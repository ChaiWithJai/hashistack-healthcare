//! Contract tests for the audit broker (#8): no audit write, no operation.
//!
//! The issue's bar, verbatim: kill the audit sink; the next promotion must
//! fail with an audit-unavailable error, not succeed silently. Plus the
//! fallback invariant (the record of the failure itself is never lost) and
//! the salted-HMAC boundary (tenant view plaintext, platform export and
//! durable archive `hmac-sha256:` only).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::Notify;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use rust_proof_service::api;
use rust_proof_service::audit::{AuditEvent, AuditSink, Broker, FileSink, SinkFuture};
use rust_proof_service::deploy::{CleanupDriver, CleanupFailure};
use rust_proof_service::packs;
use rust_proof_service::state::{Allocation, Platform, SharedPlatform, Stage};

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

struct BarrierSink {
    armed: AtomicBool,
    confirmed: AtomicU64,
    started: Notify,
    release: Notify,
}

struct FailThenSucceedCleanup {
    calls: AtomicUsize,
    retry_saw_confirmed_stop: AtomicBool,
}

struct BlockingCleanup {
    started: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

struct FailOnAppendSink {
    calls: AtomicUsize,
    confirmed: AtomicU64,
    fail_on: usize,
}

impl AuditSink for FailOnAppendSink {
    fn name(&self) -> &'static str {
        "fail-on-append"
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
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.fail_on {
                anyhow::bail!("injected checkpoint audit failure");
            }
            if let Some(last) = events.last() {
                self.confirmed.fetch_max(last.seq, Ordering::SeqCst);
            }
            Ok(())
        })
    }
}

impl CleanupDriver for BlockingCleanup {
    fn rollback(
        &self,
        _snapshot: &rust_proof_service::state::AppRecord,
    ) -> Result<Vec<(String, String)>, CleanupFailure> {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        self.release.lock().unwrap().recv().unwrap();
        Ok(Vec::new())
    }
}

impl CleanupDriver for FailThenSucceedCleanup {
    fn rollback(
        &self,
        snapshot: &rust_proof_service::state::AppRecord,
    ) -> Result<Vec<(String, String)>, CleanupFailure> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            return Err(CleanupFailure::injected(
                true,
                "vault-token-secret must never reach the app record",
            ));
        }
        self.retry_saw_confirmed_stop.store(
            snapshot
                .allocation
                .as_ref()
                .is_some_and(|allocation| allocation.cleanup_workload_stopped),
            Ordering::SeqCst,
        );
        Ok(vec![(
            "vault.lease_revoked".into(),
            "injected cleanup verification succeeded".into(),
        )])
    }
}

impl BarrierSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            armed: AtomicBool::new(false),
            confirmed: AtomicU64::new(0),
            started: Notify::new(),
            release: Notify::new(),
        })
    }
    fn arm_failure(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
}

impl AuditSink for BarrierSink {
    fn name(&self) -> &'static str {
        "barrier"
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
            if self.armed.swap(false, Ordering::SeqCst) {
                self.started.notify_one();
                self.release.notified().await;
                anyhow::bail!("injected delayed sink failure");
            }
            if let Some(last) = events.last() {
                self.confirmed.fetch_max(last.seq, Ordering::SeqCst);
            }
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

fn cleanup_allocation(stopped: bool) -> Allocation {
    Allocation {
        id: "alloc-cleanup".into(),
        pool: "prod".into(),
        region: "nyc3".into(),
        image: "example.invalid/app@sha256:test".into(),
        profile: "shared-small".into(),
        database: "vault-dynamic".into(),
        credentials: "vault-lease".into(),
        app_version: 1,
        url: "https://example.invalid".into(),
        healthy: false,
        cleanup_pending: true,
        cleanup_workload_stopped: stopped,
        cleanup_error: Some("credential-cleanup-failed".into()),
        deployed_at: 1,
        nomad_eval_id: None,
        vault_transit_key: None,
        vault_lease_id: None,
        vault_db_username: None,
        vault_lease_ttl_secs: None,
    }
}

fn actions(audit: &Value) -> Vec<&str> {
    audit["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn same_app_mutations_serialize_through_durable_confirmation() {
    let sink = BarrierSink::new();
    let (_platform, router) = platform_with_sink(sink.clone()).await;
    let id = create_app(&router).await;
    sink.arm_failure();

    let first_router = router.clone();
    let first_id = id.clone();
    let first = tokio::spawn(async move {
        call(
            &first_router,
            "POST",
            &format!("/api/apps/{first_id}/gate/auto-logoff/fix"),
            Some(json!({})),
        )
        .await
    });
    sink.started.notified().await;

    let second_router = router.clone();
    let second_id = id.clone();
    let second = tokio::spawn(async move {
        call(
            &second_router,
            "POST",
            &format!("/api/apps/{second_id}/iterate"),
            Some(json!({"instruction": "add caregiver SMS reminders"})),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !second.is_finished(),
        "later mutation must wait for the earlier durable verdict"
    );

    sink.release.notify_one();
    let (first_status, _) = first.await.unwrap();
    assert_eq!(first_status, StatusCode::SERVICE_UNAVAILABLE);
    let (second_status, second_body) = second.await.unwrap();
    assert_eq!(second_status, StatusCode::OK, "{second_body}");

    let (_, app) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    let controls = app["controls"].as_array().unwrap();
    assert!(
        !controls.iter().any(|control| control == "auto-logoff"),
        "failed first mutation must not survive inside the later success: {app}"
    );
    assert!(
        app["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|feature| feature == "add caregiver SMS reminders"),
        "later mutation should apply after the failed mutation is reverted: {app}"
    );
}

#[tokio::test]
async fn cleanup_pending_refuses_edits_and_reports_withdrawal_honestly() {
    let sink = KillableSink::new();
    let (platform, router) = platform_with_sink(sink).await;
    let id = create_app(&router).await;
    {
        let mut state = platform.write().unwrap();
        let app = state.apps.get_mut(&id).unwrap();
        app.stage = Stage::Live;
        app.allocation = Some(cleanup_allocation(false));
    }

    for (method, path, body) in [
        (
            "POST",
            format!("/api/apps/{id}/iterate"),
            Some(json!({"instruction":"change medication workflow"})),
        ),
        (
            "POST",
            format!("/api/apps/{id}/gate/auto-logoff/fix"),
            Some(json!({})),
        ),
        ("POST", format!("/api/apps/{id}/review"), Some(json!({}))),
        (
            "POST",
            format!("/api/apps/{id}/promote"),
            Some(json!({"synthetic_demo":true})),
        ),
        ("GET", format!("/api/apps/{id}/export"), None),
    ] {
        let (status, response) = call(&router, method, &path, body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{path}: {response}");
    }

    let (status, operating) = call(&router, "GET", &format!("/api/apps/{id}/operate"), None).await;
    assert_eq!(status, StatusCode::OK, "{operating}");
    assert_eq!(operating["desired_state"], "stopped");
    assert_eq!(operating["status_source"], "simulated");

    platform
        .write()
        .unwrap()
        .apps
        .get_mut(&id)
        .unwrap()
        .allocation
        .as_mut()
        .unwrap()
        .cleanup_workload_stopped = true;
    let (_, operating) = call(&router, "GET", &format!("/api/apps/{id}/operate"), None).await;
    assert_eq!(operating["desired_state"], "stopped");
    assert_eq!(operating["observed_state"], "stopped");
    assert_eq!(operating["status_source"], "rollback-cleanup");
}

#[tokio::test]
async fn rollback_persists_stopped_cleanup_and_retry_completes_without_secret_leakage() {
    let sink = KillableSink::new();
    let (platform, router) = platform_with_sink(sink).await;
    let driver = Arc::new(FailThenSucceedCleanup {
        calls: AtomicUsize::new(0),
        retry_saw_confirmed_stop: AtomicBool::new(false),
    });
    platform.write().unwrap().cleanup_driver = driver.clone();
    let id = create_app(&router).await;
    {
        let mut state = platform.write().unwrap();
        let app = state.apps.get_mut(&id).unwrap();
        app.stage = Stage::Live;
        let mut allocation = cleanup_allocation(false);
        allocation.cleanup_pending = false;
        allocation.cleanup_error = None;
        app.allocation = Some(allocation);
    }

    let (status, response) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/rollback"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{response}");
    let (_, pending) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    assert_eq!(pending["stage"], "live");
    assert_eq!(pending["allocation"]["cleanup_pending"], true);
    assert_eq!(pending["allocation"]["cleanup_workload_stopped"], true);
    assert_eq!(
        pending["allocation"]["cleanup_error"],
        "credential-cleanup-failed"
    );
    assert!(
        !pending.to_string().contains("vault-token-secret"),
        "backend details must not cross the API boundary: {pending}"
    );
    let restarted: rust_proof_service::state::AppRecord =
        serde_json::from_value(pending.clone()).unwrap();
    let restarted_allocation = restarted.allocation.unwrap();
    assert!(restarted_allocation.cleanup_pending);
    assert!(restarted_allocation.cleanup_workload_stopped);

    let (status, response) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/rollback"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["stage"], "sandbox");
    assert!(response["allocation"].is_null());
    assert!(driver.retry_saw_confirmed_stop.load(Ordering::SeqCst));
    assert_eq!(driver.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn blocked_cleanup_does_not_stop_a_different_app_from_progressing() {
    let sink = KillableSink::new();
    let (platform, router) = platform_with_sink(sink).await;
    let first_id = create_app(&router).await;
    let second_id = create_app(&router).await;
    {
        let mut state = platform.write().unwrap();
        let app = state.apps.get_mut(&first_id).unwrap();
        app.stage = Stage::Live;
        let mut allocation = cleanup_allocation(false);
        allocation.cleanup_pending = false;
        allocation.cleanup_error = None;
        app.allocation = Some(allocation);
    }
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    platform.write().unwrap().cleanup_driver = Arc::new(BlockingCleanup {
        started: Mutex::new(Some(started_tx)),
        release: Mutex::new(release_rx),
    });

    let rollback_router = router.clone();
    let rollback_id = first_id.clone();
    let rollback = tokio::spawn(async move {
        call(
            &rollback_router,
            "POST",
            &format!("/api/apps/{rollback_id}/rollback"),
            None,
        )
        .await
    });
    tokio::task::spawn_blocking(move || started_rx.recv().unwrap())
        .await
        .unwrap();

    let independent = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        call(
            &router,
            "POST",
            &format!("/api/apps/{second_id}/iterate"),
            Some(json!({"instruction":"add caregiver status summaries"})),
        ),
    )
    .await
    .expect("a different app must not wait on external cleanup");
    assert_eq!(independent.0, StatusCode::OK, "{}", independent.1);

    release_tx.send(()).unwrap();
    let (status, response) = rollback.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{response}");
}

#[tokio::test]
async fn final_transition_waits_for_a_durable_verified_cleanup_checkpoint() {
    let sink = Arc::new(FailOnAppendSink {
        calls: AtomicUsize::new(0),
        confirmed: AtomicU64::new(0),
        fail_on: 3,
    });
    let (platform, router) = platform_with_sink(sink).await;
    let driver = Arc::new(FailThenSucceedCleanup {
        calls: AtomicUsize::new(1),
        retry_saw_confirmed_stop: AtomicBool::new(false),
    });
    platform.write().unwrap().cleanup_driver = driver.clone();
    let id = create_app(&router).await;
    {
        let mut state = platform.write().unwrap();
        let app = state.apps.get_mut(&id).unwrap();
        app.stage = Stage::Live;
        let mut allocation = cleanup_allocation(false);
        allocation.cleanup_pending = false;
        allocation.cleanup_error = None;
        app.allocation = Some(allocation);
    }

    let (status, response) = call(&router, "POST", &format!("/api/apps/{id}/rollback"), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{response}");
    let (_, pending) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    assert_eq!(pending["stage"], "live");
    assert_eq!(pending["allocation"]["cleanup_pending"], true);
    assert_eq!(pending["allocation"]["cleanup_workload_stopped"], true);
    assert!(pending["allocation"]["cleanup_error"].is_null());

    let (status, response) = call(&router, "POST", &format!("/api/apps/{id}/rollback"), None).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(driver.retry_saw_confirmed_stop.load(Ordering::SeqCst));
}

#[tokio::test]
async fn real_cleanup_never_runs_without_a_control_store() {
    let sink = KillableSink::new();
    let (platform, router) = platform_with_sink(sink).await;
    let driver = Arc::new(FailThenSucceedCleanup {
        calls: AtomicUsize::new(0),
        retry_saw_confirmed_stop: AtomicBool::new(false),
    });
    platform.write().unwrap().cleanup_driver = driver.clone();
    let id = create_app(&router).await;
    {
        let mut state = platform.write().unwrap();
        let app = state.apps.get_mut(&id).unwrap();
        app.stage = Stage::Live;
        let mut allocation = cleanup_allocation(false);
        allocation.cleanup_pending = false;
        allocation.cleanup_error = None;
        allocation.nomad_eval_id = Some("eval-real".into());
        app.allocation = Some(allocation);
    }
    let (status, response) = call(&router, "POST", &format!("/api/apps/{id}/rollback"), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{response}");
    assert_eq!(driver.calls.load(Ordering::SeqCst), 0);
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
