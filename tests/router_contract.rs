//! Contract tests for treatment 4a — the router-in-driver composite (#4).
//!
//! The claims under test: iterate routes local-first and commits only edits
//! that don't regress the gate report; invalid or regressive local edits
//! escalate automatically (and invisibly) to the frontier driver; every
//! routing decision lands in the audit stream; and with no endpoints
//! configured the router is byte-for-byte the rule-based driver.
//!
//! Model endpoints are tiny in-process axum servers speaking the
//! OpenAI-compatible chat-completions shape on 127.0.0.1 ephemeral ports.
//! Nothing here touches the network.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use tower::ServiceExt;

use rust_proof_service::agent::RouterDriver;
use rust_proof_service::state::Platform;
use rust_proof_service::{api, packs};

// ---------- mock OpenAI-compatible model server ----------

struct MockModel {
    /// Scripted `choices[0].message.content` values, served in order.
    script: Mutex<VecDeque<String>>,
    /// Every request body received, for asserting the client's wire shape.
    requests: Mutex<Vec<Value>>,
}

async fn completions(State(mock): State<Arc<MockModel>>, Json(body): Json<Value>) -> Json<Value> {
    mock.requests.lock().unwrap().push(body);
    let content = mock
        .script
        .lock()
        .unwrap()
        .pop_front()
        .expect("mock model script exhausted");
    Json(json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion",
        "model": "mock",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }]
    }))
}

/// Spin a mock model on an ephemeral 127.0.0.1 port; returns its base URL.
async fn spawn_mock_model(script: Vec<&str>) -> (String, Arc<MockModel>) {
    let mock = Arc::new(MockModel {
        script: Mutex::new(script.into_iter().map(String::from).collect()),
        requests: Mutex::new(Vec::new()),
    });
    let router = Router::new()
        .route("/v1/chat/completions", post(completions))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, mock)
}

// ---------- control plane driven through the public API ----------

fn control_plane(driver: RouterDriver) -> Router {
    let mut plat = Platform::new(packs::builtin_packs());
    plat.agent_driver = driver;
    api::router_with_state(Arc::new(RwLock::new(plat)))
}

async fn call(
    router: &Router,
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

async fn create_post_op_app(router: &Router) -> String {
    let (status, body) = call(
        router,
        "POST",
        "/api/apps",
        Some(json!({
            "prompt": "a post-op recovery tracker for my knee replacement patients",
            "pack": "post-op-monitor",
            "name": "post-op tracker"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create failed: {body}");
    body["app"]["id"].as_str().unwrap().to_string()
}

async fn routed_details(router: &Router, id: &str) -> Vec<String> {
    let (_, audit) = call(router, "GET", &format!("/api/apps/{id}/audit"), None).await;
    audit["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "agent.routed")
        .map(|e| {
            assert_eq!(e["actor"], "agent-router");
            e["detail"].as_str().unwrap().to_string()
        })
        .collect()
}

// ---------- the contracts ----------

/// Local success path: a valid, non-regressive local edit is committed, the
/// audit stream shows "iterate v2 → local … ok", and the wire shape the
/// client sent is a real chat-completions request.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_routes_local_first_and_audits_the_decision() {
    let (local_url, mock) = spawn_mock_model(vec![
        r#"{"summary":"pain 0-10 scale with alert flag","wire_controls":["escalation-path"],"drop_controls":[]}"#,
    ])
    .await;
    let router = control_plane(RouterDriver::new(Some(local_url), None));
    let id = create_post_op_app(&router).await;

    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "make pain a 0-10 scale and flag anything over 7 to me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["reply"]["message"]
        .as_str()
        .unwrap()
        .contains("pain 0-10 scale with alert flag"));
    assert!(body["reply"]["wired_controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "escalation-path"));
    assert_eq!(body["app"]["current_version"], 2);

    // Both routing decisions are in the audit stream: scaffold → frontier
    // (offline stub — no FRONTIER_MODEL_URL, no network), iterate → local ok.
    let routed = routed_details(&router, &id).await;
    assert!(
        routed
            .iter()
            .any(|d| d.contains("scaffold → frontier") && d.contains("stub, offline")),
        "missing scaffold routing in {routed:?}"
    );
    assert!(
        routed
            .iter()
            .any(|d| d.contains("iterate v2 → local (qwen3-coder) ok")),
        "missing local-ok routing in {routed:?}"
    );

    // The mock saw exactly one well-formed OpenAI chat-completions request.
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "qwen3-coder");
    let messages = requests[0]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert!(messages[1]["content"]
        .as_str()
        .unwrap()
        .contains("0-10 scale"));
}

/// Escalation on gate regression: the local model proposes dropping the
/// prewired audit-log control. The router preflights the candidate, sees the
/// report regress, discards the candidate, and escalates the pristine record
/// to the frontier endpoint — recording exactly why.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_gate_regression_escalates_to_frontier() {
    let (local_url, _local) = spawn_mock_model(vec![
        r#"{"summary":"strip audit middleware","wire_controls":[],"drop_controls":["audit-log"]}"#,
    ])
    .await;
    // The frontier mock answers the scaffold authorship call first, then the
    // escalated edit.
    let (frontier_url, frontier) = spawn_mock_model(vec![
        "scaffold plan confirmed",
        r#"{"summary":"pain scale with flag rule","wire_controls":["escalation-path"],"drop_controls":[]}"#,
    ])
    .await;
    let router = control_plane(RouterDriver::new(Some(local_url), Some(frontier_url)));
    let id = create_post_op_app(&router).await;

    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "make pain a 0-10 scale and flag anything over 7 to me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The frontier edit landed; the regressive local candidate did not.
    let features = body["app"]["features"].as_array().unwrap();
    assert!(features.iter().any(|f| f == "pain scale with flag rule"));
    assert!(!features.iter().any(|f| f == "strip audit middleware"));
    let controls = body["app"]["controls"].as_array().unwrap();
    assert!(
        controls.iter().any(|c| c == "audit-log"),
        "the discarded local edit must not cost the app its audit-log control"
    );

    let routed = routed_details(&router, &id).await;
    let escalation = routed
        .iter()
        .find(|d| d.starts_with("iterate v2"))
        .expect("iterate decision must be audited");
    assert!(
        escalation.contains("local (qwen3-coder) failed gate-regression (5/6 → 4/6)"),
        "escalation must record why: {escalation}"
    );
    assert!(
        escalation.contains("→ escalated frontier (claude-frontier) ok"),
        "escalation must record where: {escalation}"
    );

    // The frontier endpoint really was called twice: scaffold + escalation.
    assert_eq!(frontier.requests.lock().unwrap().len(), 2);
}

/// Escalation on an invalid reply: the local model answers with prose
/// instead of the edit protocol. No frontier URL is configured, so the
/// frontier stub degrades to the deterministic rule-based edit — the
/// doctor's instruction still lands, and the audit trail says how.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_invalid_reply_escalates_to_offline_frontier_stub() {
    let (local_url, _mock) =
        spawn_mock_model(vec!["I'm sorry, I can't produce structured edits."]).await;
    let router = control_plane(RouterDriver::new(Some(local_url), None));
    let id = create_post_op_app(&router).await;

    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "flag anything over 7 to me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Rule-based fallback semantics: "flag" wires the escalation path.
    assert!(body["reply"]["wired_controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "escalation-path"));

    let routed = routed_details(&router, &id).await;
    let escalation = routed
        .iter()
        .find(|d| d.starts_with("iterate v2"))
        .expect("iterate decision must be audited");
    assert!(escalation.contains("failed invalid-reply"), "{escalation}");
    assert!(
        escalation.contains("escalated frontier (claude-frontier stub, offline) ok"),
        "{escalation}"
    );
}

/// Escalation on transport failure: the local endpoint is a dead port. The
/// edit still lands (offline frontier stub → rule-based) and the audit
/// stream records the transport failure as the reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_transport_failure_escalates() {
    // Bind then drop to get a 127.0.0.1 port with nothing listening.
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let router = control_plane(RouterDriver::new(Some(dead), None));
    let id = create_post_op_app(&router).await;

    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "add a wound photo comparison view"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["app"]["current_version"], 2);

    let routed = routed_details(&router, &id).await;
    let escalation = routed
        .iter()
        .find(|d| d.starts_with("iterate v2"))
        .expect("iterate decision must be audited");
    assert!(escalation.contains("failed transport"), "{escalation}");
    assert!(escalation.contains("escalated frontier"), "{escalation}");
}

/// No endpoints configured → the router is the rule-based driver, exactly:
/// same replies, same state, and zero agent.routed events. This is what lets
/// every pre-existing test (and CI, and offline dev) pass unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unconfigured_router_is_rule_based_passthrough_with_no_routing_events() {
    let router = control_plane(RouterDriver::new(None, None));
    let id = create_post_op_app(&router).await;

    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "make pain a 0-10 scale and flag anything over 7 to me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Rule-based semantics, verbatim.
    assert!(body["reply"]["message"].as_str().unwrap().contains(
        "✓ done — make pain a 0-10 scale and flag anything over 7 to me. \
         Also wired: escalation-path."
    ));

    let routed = routed_details(&router, &id).await;
    assert!(
        routed.is_empty(),
        "passthrough mode must emit no routing events, got {routed:?}"
    );
}
