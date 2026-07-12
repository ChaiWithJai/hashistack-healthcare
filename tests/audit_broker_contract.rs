//! Contract tests for the audit broker (#8): no audit write, no operation.
//!
//! The issue's bar, verbatim: kill the audit sink; the next promotion must
//! fail with an audit-unavailable error, not succeed silently. Plus the
//! fallback invariant (the record of the failure itself is never lost) and
//! the salted-HMAC boundary (tenant view plaintext, platform export and
//! durable archive `hmac-sha256:` only).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use rust_proof_service::api;
use rust_proof_service::audit::{AuditEvent, AuditSink, Broker, FileSink, SinkFuture};
use rust_proof_service::packs;
use rust_proof_service::state::{Platform, SharedPlatform};

const PROMPT: &str = "a post-op recovery tracker for my knee replacement patients";

// ---------- a durable sink that can be killed mid-flight ----------

/// Passes its registration probe, then fails on demand — the "disk pulled
/// out from under the pipeline" double the issue's bar asks for.
struct KillableSink {
    dead: AtomicBool,
    confirmed: AtomicU64,
    received: Mutex<Vec<AuditEvent>>,
}

impl KillableSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            dead: AtomicBool::new(false),
            confirmed: AtomicU64::new(0),
            received: Mutex::new(Vec::new()),
        })
    }
    fn kill(&self) {
        self.dead.store(true, Ordering::SeqCst);
    }
    fn revive(&self) {
        self.dead.store(false, Ordering::SeqCst);
    }
    fn received_actions(&self) -> Vec<String> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.action.clone())
            .collect()
    }
}

impl AuditSink for KillableSink {
    fn name(&self) -> &'static str {
        "killable"
    }
    fn durable(&self) -> bool {
        true
    }
    fn confirmed_seq(&self) -> u64 {
        self.confirmed.load(Ordering::SeqCst)
    }
    fn probe(&self) -> SinkFuture<'_> {
        Box::pin(async { Ok(()) })
    }
    fn append<'a>(&'a self, events: &'a [AuditEvent]) -> SinkFuture<'a> {
        Box::pin(async move {
            if self.dead.load(Ordering::SeqCst) {
                anyhow::bail!("sink medium gone (killed by test)");
            }
            let since = self.confirmed.load(Ordering::SeqCst);
            let mut received = self.received.lock().unwrap();
            let mut max = since;
            for e in events.iter().filter(|e| e.seq > since) {
                received.push(e.clone());
                max = max.max(e.seq);
            }
            self.confirmed.fetch_max(max, Ordering::SeqCst);
            Ok(())
        })
    }
}

// ---------- harness ----------

async fn platform_with_sink(sink: Arc<dyn AuditSink>) -> (SharedPlatform, axum::Router) {
    let mut platform = Platform::new(packs::builtin_packs());
    let mut broker = Broker::new();
    broker
        .register(sink)
        .await
        .expect("the test sink passes its registration probe");
    platform.broker = Arc::new(broker);
    let shared: SharedPlatform = Arc::new(RwLock::new(platform));
    let router = api::router_with_state(shared.clone());
    (shared, router)
}

async fn call(
    router: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = match body {
        Some(v) => Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    };
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn call_text(router: &axum::Router, uri: &str) -> String {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn create_app(router: &axum::Router) -> String {
    let (status, body) = call(
        router,
        "POST",
        "/api/apps",
        Some(json!({"prompt": PROMPT, "pack": "post-op-monitor", "name": "post-op tracker"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create failed: {body}");
    body["app"]["id"].as_str().unwrap().to_string()
}

fn actions(audit: &Value) -> Vec<&str> {
    audit["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect()
}

// ---------- the issue's bar ----------

#[tokio::test]
async fn kill_the_sink_and_the_next_promotion_fails_sandboxed_with_the_failure_on_record() {
    let sink = KillableSink::new();
    let (_platform, router) = platform_with_sink(sink.clone()).await;

    // Healthy sink: the doctor's flow works and the sink confirms it.
    let id = create_app(&router).await;
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(sink.received_actions().contains(&"gate.fixed".to_string()));

    // Kill it. The next promotion must fail loudly, not succeed silently.
    sink.kill();
    let (status, err) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "promotion without a durable audit write must 503: {err}"
    );
    assert!(
        err["error"].as_str().unwrap().contains("audit unavailable"),
        "the error names the audit pipeline: {err}"
    );

    // State did not change: still sandboxed, no allocation, no attestation.
    let (_, app) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    assert_eq!(app["stage"], "sandbox");
    assert!(app["allocation"].is_null());
    assert!(app["attestation"].is_null());

    // The fallback invariant: the failure itself is on record in memory —
    // both the sink failure and the reverted promotion.
    let (_, audit) = call(&router, "GET", &format!("/api/apps/{id}/audit"), None).await;
    let acts = actions(&audit);
    assert!(acts.contains(&"audit.sink_failed"), "{acts:?}");
    assert!(acts.contains(&"app.promotion_reverted"), "{acts:?}");
    assert!(
        !acts.contains(&"app.promoted") || app["stage"] == "sandbox",
        "no silent success"
    );

    // Revive the sink: the promotion goes through, and the durable sink
    // catches up on everything it missed — including the failure record.
    sink.revive();
    let (status, live) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{live}");
    assert_eq!(live["app"]["stage"], "live");
    let caught_up = sink.received_actions();
    assert!(
        caught_up.contains(&"audit.sink_failed".to_string()),
        "{caught_up:?}"
    );
    assert!(caught_up.contains(&"app.promotion_reverted".to_string()));
    assert!(caught_up.contains(&"app.promoted".to_string()));
}

#[tokio::test]
async fn a_dead_sink_blocks_every_load_bearing_operation_not_just_promote() {
    let sink = KillableSink::new();
    let (_platform, router) = platform_with_sink(sink.clone()).await;
    let id = create_app(&router).await;
    sink.kill();

    // gate fix reverts: the control is not wired.
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (_, app) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    assert!(
        !app["controls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "auto-logoff"),
        "the unrecorded fix must not stand: {app}"
    );

    // iterate reverts: the applied addendum is withdrawn.
    let before = app["current_version"].as_u64().unwrap();
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "remind patients to log wound photos"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (_, app) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    assert_eq!(app["current_version"].as_u64().unwrap(), before);

    // export is withheld: the bundle never leaves without a durable record.
    let (status, _) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    // and a brand-new draft is withdrawn entirely.
    let (status, err) = call(
        &router,
        "POST",
        "/api/apps",
        Some(json!({"prompt": "x", "pack": "post-op-monitor", "name": "ghost"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{err}");
    let (_, apps) = call(&router, "GET", "/api/apps", None).await;
    assert_eq!(
        apps["apps"].as_array().unwrap().len(),
        1,
        "the unauditable draft must not exist"
    );

    // Reads stay best-effort: the doctor can still SEE everything.
    let (status, _) = call(&router, "GET", &format!("/api/apps/{id}/gate"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&router, "GET", &format!("/api/apps/{id}/audit"), None).await;
    assert_eq!(status, StatusCode::OK);
}

// ---------- the HMAC boundary, end to end over a real durable sink ----------

#[tokio::test]
async fn file_sink_archives_hmac_form_while_the_doctor_keeps_their_words() {
    let path = std::env::temp_dir().join(format!(
        "audit-broker-contract-{}.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let sink = Arc::new(FileSink::open(&path, 0).expect("open"));
    let (_platform, router) = platform_with_sink(sink).await;

    let id = create_app(&router).await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "remind patients to log their wound photos daily"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The durable archive: fsync'd JSONL, probe line + every event, and the
    // doctor's words appear ONLY as hmac-sha256:.
    let archive = std::fs::read_to_string(&path).unwrap();
    assert!(archive.contains("audit.sink_probe"));
    assert!(archive.contains("app.promoted"));
    assert!(archive.contains("hmac-sha256:"), "{archive}");
    assert!(
        !archive.contains("knee replacement"),
        "platform archive must not disclose the prompt"
    );
    assert!(!archive.contains("wound photos"), "nor the instruction");

    // The platform-wide export: same boundary.
    let export = call_text(&router, "/api/audit/export").await;
    assert!(export.contains("hmac-sha256:"));
    assert!(!export.contains("knee replacement"));

    // The doctor's own app-scoped view: their own words, in plaintext.
    let (_, audit) = call(&router, "GET", &format!("/api/apps/{id}/audit"), None).await;
    let events = audit["events"].as_array().unwrap();
    let created = events
        .iter()
        .find(|e| e["action"] == "app.created")
        .unwrap();
    assert_eq!(created["sensitive"]["prompt"], PROMPT);
    let iterated = events
        .iter()
        .find(|e| e["action"] == "app.iterated")
        .unwrap();
    assert_eq!(
        iterated["sensitive"]["instruction"],
        "remind patients to log their wound photos daily"
    );

    // And the ejected COMPLIANCE.md — the doctor's own record — keeps the
    // plaintext (decision 0004).
    let (status, bundle) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    assert_eq!(status, StatusCode::OK);
    let compliance = bundle["files"]["docs/COMPLIANCE.md"].as_str().unwrap();
    assert!(compliance.contains("knee replacement"), "{compliance}");
    assert!(!compliance.contains("hmac-sha256:"), "{compliance}");

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn dev_mode_without_a_durable_sink_never_blocks() {
    // No broker injection: Platform::new is dev mode (memory fallback only).
    let router = rust_proof_service::app();
    let id = create_app(&router).await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    let (status, live) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{live}");
    assert_eq!(live["app"]["stage"], "live");
}
