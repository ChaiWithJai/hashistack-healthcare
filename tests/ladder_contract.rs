//! Contract tests for the verified escalation ladder (#4, decision 0001).
//!
//! The claims under test: every agent action is a Waypoint-style operation
//! upserted Running BEFORE any driver runs (crash-visible by construction);
//! routing emerges from the verifier's verdicts, not from prediction; the
//! pack's signed routing policy picks each action's first tier and consents
//! (or not) to frontier escalation; and a full-ladder failure leaves the
//! app record untouched.
//!
//! Model tiers speak OpenAI-compatible HTTP to mock servers on ephemeral
//! loopback ports (decision 0002 mock tier). Nothing here ever touches the
//! real network — the harness and the client both refuse non-loopback URLs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;
use tower::ServiceExt;

use rust_proof_service::agent::{
    AgentDriver, AgentReply, HttpModelDriver, RuleBasedDriver, ScaffoldStep,
};
use rust_proof_service::api;
use rust_proof_service::ladder::EscalationLadder;
use rust_proof_service::packs::{self, PackManifest, RoutingPolicy, RoutingTier};
use rust_proof_service::state::{AppRecord, DataSource, OpStatus, Platform, SharedPlatform, Stage};

// ---------- helpers ----------

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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn shared_platform() -> SharedPlatform {
    Arc::new(RwLock::new(Platform::new(packs::builtin_packs())))
}

async fn create_app(router: &axum::Router, pack: &str) -> String {
    let (status, body) = call(
        router,
        "POST",
        "/api/apps",
        Some(json!({
            "prompt": "a post-op recovery tracker for my knee replacement patients",
            "pack": pack,
            "name": format!("{pack} app")
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create failed: {body}");
    body["app"]["id"].as_str().unwrap().to_string()
}

async fn iterate(router: &axum::Router, id: &str, instruction: &str) -> (StatusCode, Value) {
    call(
        router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({ "instruction": instruction })),
    )
    .await
}

async fn fix_auto_logoff(router: &axum::Router, id: &str) {
    call(
        router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
}

/// The iterate operation's row from GET /api/apps/:id/operations.
async fn iterate_op(router: &axum::Router, id: &str) -> Value {
    let (status, ops) = call(router, "GET", &format!("/api/apps/{id}/operations"), None).await;
    assert_eq!(status, StatusCode::OK);
    ops["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == "iterate")
        .expect("an iterate operation exists")
        .clone()
}

/// All audit details for one app, filtered to one action.
async fn audit_details(router: &axum::Router, id: &str, action: &str) -> Vec<String> {
    let (_, audit) = call(router, "GET", &format!("/api/apps/{id}/audit"), None).await;
    audit["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == action)
        .map(|e| e["detail"].as_str().unwrap().to_string())
        .collect()
}

/// `(tier, verdict)` pairs from an operation's attempt list.
fn attempt_pairs(op: &Value) -> Vec<(String, String)> {
    op["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["tier"].as_str().unwrap().to_string(),
                a["verdict"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Mock OpenAI-compatible chat-completions server on an ephemeral loopback
/// port. Every request gets a 200 whose choices[0].message.content is
/// `content`; the returned counter records how many requests arrived.
fn mock_model_server(content: String) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap();
    // Decision 0002: no test path may ever bill a real API — the harness
    // refuses to hand out anything but a loopback model URL.
    assert!(
        addr.ip().is_loopback(),
        "mock model server must be loopback"
    );
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = hits.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut socket) = stream else { break };
            server_hits.fetch_add(1, Ordering::SeqCst);
            let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
            read_http_request(&mut socket);
            let body = json!({
                "object": "chat.completion",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": content}}]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}"), hits)
}

/// Consume one HTTP request (headers + content-length body) off the socket.
fn read_http_request(socket: &mut TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut header_end: Option<usize> = None;
    let mut content_len = 0usize;
    loop {
        if header_end.is_none() {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = Some(pos + 4);
                let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                content_len = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
            }
        }
        if let Some(end) = header_end {
            if buf.len() >= end + content_len {
                return;
            }
        }
        match socket.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

/// An edit that drops auto-logoff — a gate regression the verifier catches.
fn regressing_edit() -> String {
    json!({
        "feature": "session keep-alive",
        "drop_controls": ["auto-logoff"],
        "message": "kept sessions open for convenience"
    })
    .to_string()
}

fn local_tier(url: String) -> (String, Box<dyn AgentDriver>) {
    ("local".to_string(), Box::new(HttpModelDriver::local(url)))
}

fn frontier_tier(url: String) -> (String, Box<dyn AgentDriver>) {
    (
        "frontier".to_string(),
        Box::new(HttpModelDriver::frontier(url)),
    )
}

/// A tier that crashes mid-flight — the operation row upserted before it ran
/// must survive as the Running record of the interrupted action.
struct PanickingDriver;

impl AgentDriver for PanickingDriver {
    fn scaffold(&self, _pack: &PackManifest, _prompt: &str) -> Vec<ScaffoldStep> {
        panic!("driver crashed before producing a scaffold")
    }
    fn iterate(&self, _app: &mut AppRecord, _i: &str, _g: &[String]) -> AgentReply {
        panic!("driver crashed mid-edit")
    }
}

fn seed_app(plat: &mut Platform) -> String {
    let id = "seed-app".to_string();
    plat.apps.insert(
        id.clone(),
        AppRecord {
            id: id.clone(),
            name: "seed".to_string(),
            prompt: "a seeded app".to_string(),
            pack: "post-op-monitor".to_string(),
            stage: Stage::Sandbox,
            data_source: DataSource::Synthetic("synthea-postop-v1".to_string()),
            controls: BTreeSet::new(),
            external_calls: Vec::new(),
            features: vec!["symptom check-in".to_string()],
            routes: 1,
            addenda: Vec::new(),
            current_version: 1,
            reviewer_note: None,
            allocation: None,
            attestation: None,
            tenant: "meridian".to_string(),
        },
    );
    id
}

// ---------- the default ladder: rules accepts, everything is an operation ----------

#[tokio::test]
async fn ladder_accepts_at_rules_tier_by_default() {
    let platform = shared_platform();
    let router = api::router_with_state(platform.clone());
    let id = create_app(&router, "post-op-monitor").await;

    let (status, _) = iterate(&router, &id, "flag any fever over 101 to me").await;
    assert_eq!(status, StatusCode::OK);
    fix_auto_logoff(&router, &id).await;

    let (status, ops) = call(&router, "GET", &format!("/api/apps/{id}/operations"), None).await;
    assert_eq!(status, StatusCode::OK);
    let operations = ops["operations"].as_array().unwrap();
    let kinds: Vec<&str> = operations
        .iter()
        .map(|o| o["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["scaffold", "iterate", "fix"]);
    for op in operations {
        assert_eq!(op["status"], "success", "op not settled: {op}");
        assert_eq!(op["app_id"], id.as_str());
        assert_eq!(
            attempt_pairs(op),
            vec![("rules".to_string(), "accepted".to_string())],
            "rules tier accepts on the first rung"
        );
        assert!(op["finished_at"].as_u64().is_some());
    }

    // With no model env the policy tier resolves to rules, and the audit
    // stream says so, citing the policy source (decision 0001).
    let routed = audit_details(&router, &id, "agent.routed").await;
    assert!(
        routed.iter().any(|d| d.contains(
            "per platform default routing (pack post-op-monitor declares none): \
             iterate→local (local unconfigured — resolved to rules)"
        )),
        "routing decision must cite its policy source: {routed:?}"
    );
}

#[tokio::test]
async fn operations_endpoint_404s_for_unknown_app() {
    let router = api::router_with_state(shared_platform());
    let (status, _) = call(&router, "GET", "/api/apps/nope/operations", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------- crash-visibility: upsert Running BEFORE the driver runs ----------

#[test]
fn operation_is_upserted_running_before_the_driver_runs() {
    let mut plat = Platform::new(packs::builtin_packs());
    let id = seed_app(&mut plat);
    let pack = plat.pack("post-op-monitor").unwrap().clone();
    let ladder = EscalationLadder::with_tiers(vec![(
        "rules".to_string(),
        Box::new(PanickingDriver) as Box<dyn AgentDriver>,
    )]);

    let crashed = catch_unwind(AssertUnwindSafe(|| {
        let _ = ladder.run_iterate(&mut plat, &id, "add a wound photo view", &pack);
    }));
    assert!(crashed.is_err(), "the driver must actually panic");

    // The interrupted action left its evidence: a Running row, no attempts,
    // no terminal status — exactly what a post-crash sweep would find.
    let ops = plat.operations_for_app(&id);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].status, OpStatus::Running);
    assert!(ops[0].attempts.is_empty());
    assert!(ops[0].finished_at.is_none());
    // And the app record was never touched.
    assert_eq!(plat.apps[&id].features.len(), 1);
}

// ---------- decision 0002: tests refuse non-loopback model endpoints ----------

#[test]
fn debug_builds_refuse_non_loopback_model_endpoints() {
    let mut plat = Platform::new(packs::builtin_packs());
    let id = seed_app(&mut plat);
    let driver = HttpModelDriver::local("http://198.51.100.7:9".to_string());
    let reply = driver.iterate(plat.apps.get_mut(&id).unwrap(), "add anything", &[]);
    assert!(
        reply.message.contains("non-loopback"),
        "client must refuse off-loopback endpoints in test builds: {}",
        reply.message
    );
}

// ---------- mock-server climbs ----------

#[tokio::test]
async fn policy_first_tier_is_local_and_a_verified_edit_stops_the_climb() {
    let platform = shared_platform();
    let router = api::router_with_state(platform.clone());
    let id = create_app(&router, "post-op-monitor").await;

    let (local_url, local_hits) = mock_model_server(
        json!({
            "feature": "pain scale 0-10",
            "controls": ["escalation-path"],
            "message": "wired a 0-10 pain scale"
        })
        .to_string(),
    );
    let (frontier_url, frontier_hits) =
        mock_model_server(json!({"feature": "never asked", "message": "frontier"}).to_string());
    platform.write().unwrap().ladder = Arc::new(EscalationLadder::with_tiers(vec![
        local_tier(local_url),
        frontier_tier(frontier_url),
    ]));

    let (status, body) = iterate(
        &router,
        &id,
        "make pain a 0-10 scale and flag anything over 7",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reply"]["added_feature"], "pain scale 0-10");
    assert!(body["app"]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "escalation-path"));
    assert_eq!(body["app"]["current_version"], 2);

    // The attempt history is the routing record: the pack's default policy
    // sent iterate straight to the local tier, whose verified edit ended
    // the climb — the frontier tier was never asked.
    let op = iterate_op(&router, &id).await;
    assert_eq!(op["status"], "success");
    assert_eq!(
        attempt_pairs(&op),
        vec![("local".to_string(), "accepted".to_string())]
    );
    assert_eq!(local_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        frontier_hits.load(Ordering::SeqCst),
        0,
        "a verified local edit must never reach the frontier tier"
    );
}

#[tokio::test]
async fn local_gate_regression_climbs_to_frontier() {
    let platform = shared_platform();
    let router = api::router_with_state(platform.clone());
    let id = create_app(&router, "post-op-monitor").await;
    // Wire auto-logoff so the local tier has a green check to break.
    fix_auto_logoff(&router, &id).await;

    let (local_url, _) = mock_model_server(regressing_edit());
    let (frontier_url, frontier_hits) = mock_model_server(
        json!({
            "feature": "medication reminder list",
            "message": "added the reminder list; auto-logoff untouched"
        })
        .to_string(),
    );
    platform.write().unwrap().ladder = Arc::new(EscalationLadder::with_tiers(vec![
        local_tier(local_url),
        frontier_tier(frontier_url),
    ]));

    let (status, body) = iterate(&router, &id, "add a medication reminder list").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The frontier edit landed; the regressing local edit did not.
    let features = body["app"]["features"].as_array().unwrap();
    assert!(features.iter().any(|f| f == "medication reminder list"));
    assert!(!features.iter().any(|f| f == "session keep-alive"));
    assert!(
        body["app"]["controls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "auto-logoff"),
        "the safeguard the local tier tried to drop must survive"
    );
    assert_eq!(frontier_hits.load(Ordering::SeqCst), 1);

    let op = iterate_op(&router, &id).await;
    assert_eq!(op["status"], "success");
    assert_eq!(
        attempt_pairs(&op),
        vec![
            ("local".to_string(), "rejected".to_string()),
            ("frontier".to_string(), "accepted".to_string())
        ]
    );
    assert_eq!(
        op["attempts"][0]["reason"],
        "gate-regression(auto-logoff lost)"
    );

    // The climb itself is in the audit stream, reason and all.
    let details = audit_details(&router, &id, "agent.attempt").await;
    assert!(
        details
            .iter()
            .any(|d| d.contains("tier=local verdict=gate-regression(auto-logoff lost) → climbing")),
        "audit must carry the rejection reason: {details:?}"
    );
    assert!(details
        .iter()
        .any(|d| d.contains("tier=frontier verdict=accepted → applied")));
}

#[tokio::test]
async fn pack_without_escalation_consent_gets_rules_floor_not_frontier() {
    let platform = shared_platform();
    // A signed pack that routes iterate to local but consents to NO frontier
    // escalation: the ladder must withhold frontier and land on the rules
    // floor instead (decision 0001 — pack policy is consent within the
    // ladder; the doctor's edit still lands).
    {
        let mut plat = platform.write().unwrap();
        let mut pack = plat.pack("post-op-monitor").unwrap().clone();
        pack.id = "no-consent".to_string();
        pack.routing = Some(RoutingPolicy {
            scaffold: RoutingTier::Frontier,
            iterate: RoutingTier::Local,
            review: RoutingTier::Frontier,
            escalate_on: vec![],
        });
        plat.packs.push(pack);
    }
    let router = api::router_with_state(platform.clone());
    let id = create_app(&router, "no-consent").await;
    fix_auto_logoff(&router, &id).await;

    let (local_url, _) = mock_model_server(regressing_edit());
    let (frontier_url, frontier_hits) =
        mock_model_server(json!({"feature": "never", "message": "never"}).to_string());
    platform.write().unwrap().ladder = Arc::new(EscalationLadder::with_tiers(vec![
        ("rules".to_string(), Box::new(RuleBasedDriver)),
        local_tier(local_url),
        frontier_tier(frontier_url),
    ]));

    let (status, body) = iterate(&router, &id, "add a medication reminder list").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        frontier_hits.load(Ordering::SeqCst),
        0,
        "no consent — no frontier tokens"
    );

    let op = iterate_op(&router, &id).await;
    assert_eq!(op["status"], "success");
    assert_eq!(
        attempt_pairs(&op),
        vec![
            ("local".to_string(), "rejected".to_string()),
            ("rules".to_string(), "accepted".to_string())
        ],
        "degradation floor, not frontier"
    );

    let routed = audit_details(&router, &id, "agent.routed").await;
    assert!(
        routed.iter().any(
            |d| d.contains("per pack no-consent routing policy") && d.contains("iterate→local")
        ),
        "routing decision must cite the pack's own policy: {routed:?}"
    );
    assert!(
        routed
            .iter()
            .any(|d| d.contains("not in escalate_on — frontier withheld")),
        "the withheld escalation must be auditable: {routed:?}"
    );
}

#[tokio::test]
async fn full_ladder_failure_leaves_app_untouched_and_op_failed() {
    let platform = shared_platform();
    let router = api::router_with_state(platform.clone());
    let id = create_app(&router, "post-op-monitor").await;
    let (_, before) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;

    // The only tier speaks, but not the edit protocol — every rung rejects,
    // and with no rules floor configured the operation fails terminally.
    let (local_url, _) = mock_model_server("this is prose, not a JSON edit".to_string());
    platform.write().unwrap().ladder =
        Arc::new(EscalationLadder::with_tiers(vec![local_tier(local_url)]));

    let (status, err) = iterate(&router, &id, "add a discharge summary").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(err["error"].as_str().unwrap().contains("every tier"));

    // Untouched: same version, same features, no addendum.
    let (_, after) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    assert_eq!(after["current_version"], before["current_version"]);
    assert_eq!(after["features"], before["features"]);
    assert_eq!(
        after["addenda"].as_array().unwrap().len(),
        before["addenda"].as_array().unwrap().len()
    );

    // The failure is a terminal operation row with its attempt history.
    let op = iterate_op(&router, &id).await;
    assert_eq!(op["status"], "failed");
    assert_eq!(
        attempt_pairs(&op),
        vec![("local".to_string(), "rejected".to_string())]
    );
    assert_eq!(op["attempts"][0]["reason"], "empty-edit");

    let details = audit_details(&router, &id, "agent.attempt").await;
    assert!(details.iter().any(|d| d.contains("→ failed")));
}
