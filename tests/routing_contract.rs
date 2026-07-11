//! Contract tests for pack-declared routing (treatment 4b, issue #4).
//!
//! The policy lives in the signed pack manifest; the dispatcher is
//! deliberately dumb. These tests prove: defaults resolve to the rules floor
//! when no endpoint is configured, a pack override is honored and cited,
//! `escalate_on` is the only thing that triggers escalation, and every
//! decision lands in the audit stream naming its policy source. All model
//! traffic goes to mock OpenAI-compatible servers on ephemeral local ports —
//! the network is never touched.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use rust_proof_service::agent::Dispatcher;
use rust_proof_service::packs::{builtin_packs, PackManifest};
use rust_proof_service::state::{now_unix, Addendum, AppRecord, DataSource, Stage};

// ---------- mock OpenAI-compatible server ----------

/// Serve `content` as the chat completion message content for every request,
/// counting hits. Returns the base URL and the hit counter.
fn mock_model(content: &str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let body = serde_json::json!({
        "choices": [{ "message": { "role": "assistant", "content": content } }]
    })
    .to_string();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            read_request(&mut stream);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}"), hits)
}

/// Read one HTTP request: headers, then exactly content-length body bytes.
fn read_request(stream: &mut std::net::TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let Ok(n) = stream.read(&mut chunk) else {
            return;
        };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf);
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let content_length = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if body.len() >= content_length {
                return;
            }
        }
    }
}

// ---------- fixtures ----------

fn pack(id: &str) -> PackManifest {
    builtin_packs()
        .into_iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("pack {id} exists"))
}

fn sandbox_app(pack: &PackManifest) -> AppRecord {
    AppRecord {
        id: "t4b-0001".to_string(),
        name: pack.name.clone(),
        prompt: "test app".to_string(),
        pack: pack.id.clone(),
        stage: Stage::Sandbox,
        data_source: DataSource::Synthetic(pack.synthetic_dataset.clone()),
        controls: pack.prewired.iter().cloned().collect::<BTreeSet<_>>(),
        external_calls: vec![],
        features: pack.scaffold.clone(),
        routes: pack.scaffold.len() as u32,
        addenda: vec![Addendum {
            version: 1,
            instruction: "initial draft".to_string(),
            reply: "scaffolded".to_string(),
            added_feature: None,
            wired_controls: pack.prewired.clone(),
            at: now_unix(),
        }],
        current_version: 1,
        reviewer_note: None,
        allocation: None,
        attestation: None,
        tenant: "meridian".to_string(),
    }
}

const VALID_EDIT: &str = r#"{"message":"✓ frontier applied the edit","added_feature":"payer portal link","wired_controls":[],"removed_controls":[]}"#;

// ---------- dispatcher contract ----------

#[test]
fn no_endpoints_means_every_tier_resolves_to_rules() {
    let dispatcher = Dispatcher::new(None, None);
    let pack = pack("insurance-verification");
    let mut app = sandbox_app(&pack);

    let (reply, decisions) = dispatcher.iterate(&mut app, "flag rejected claims to me", &pack);

    // Behavior is exactly the rule-based driver's: the instruction wires the
    // escalation path and the reply reads like the offline driver.
    assert!(app.controls.contains("escalation-path"));
    assert!(reply.message.starts_with("✓ done"));
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].action, "agent.routed");
    assert!(
        decisions[0].detail.contains("iterate→local")
            && decisions[0].detail.contains("resolved to rules"),
        "decision must show the unresolved tier: {}",
        decisions[0].detail
    );
}

#[test]
fn decisions_cite_pack_override_vs_platform_default() {
    let dispatcher = Dispatcher::new(None, None);

    let override_pack = pack("insurance-verification");
    let mut app = sandbox_app(&override_pack);
    let (_, decisions) = dispatcher.iterate(&mut app, "add a payer note", &override_pack);
    assert!(
        decisions[0]
            .detail
            .contains("per pack insurance-verification routing policy"),
        "override must be cited: {}",
        decisions[0].detail
    );

    let default_pack = pack("post-op-monitor");
    let mut app = sandbox_app(&default_pack);
    let (_, decisions) = dispatcher.iterate(&mut app, "add a wound photo view", &default_pack);
    assert!(
        decisions[0].detail.contains("platform default routing")
            && decisions[0].detail.contains("post-op-monitor"),
        "default must be cited: {}",
        decisions[0].detail
    );
}

#[test]
fn pack_override_routes_iterate_to_the_local_endpoint() {
    let (local_url, local_hits) = mock_model(VALID_EDIT);
    let dispatcher = Dispatcher::new(Some(local_url), None);
    let pack = pack("insurance-verification");
    let mut app = sandbox_app(&pack);

    let (reply, decisions) = dispatcher.iterate(&mut app, "add a payer portal link", &pack);

    assert_eq!(
        local_hits.load(Ordering::SeqCst),
        1,
        "local model was called"
    );
    assert_eq!(reply.message, "✓ frontier applied the edit");
    assert!(app.features.iter().any(|f| f == "payer portal link"));
    assert_eq!(decisions.len(), 1);
    assert!(decisions[0].detail.ends_with("iterate→local"));
}

#[test]
fn invalid_edit_escalates_to_frontier_per_pack_policy() {
    let (local_url, local_hits) = mock_model("sorry, I cannot produce JSON today");
    let (frontier_url, frontier_hits) = mock_model(VALID_EDIT);
    let dispatcher = Dispatcher::new(Some(local_url), Some(frontier_url));
    let pack = pack("insurance-verification"); // escalate_on includes invalid-edit
    let mut app = sandbox_app(&pack);

    let (reply, decisions) = dispatcher.iterate(&mut app, "add a payer portal link", &pack);

    assert_eq!(local_hits.load(Ordering::SeqCst), 1);
    assert_eq!(frontier_hits.load(Ordering::SeqCst), 1, "escalated once");
    assert_eq!(reply.message, "✓ frontier applied the edit");
    assert!(app.features.iter().any(|f| f == "payer portal link"));
    let escalated = decisions
        .iter()
        .find(|d| d.action == "agent.escalated")
        .expect("an escalation decision is recorded");
    assert!(
        escalated.detail.contains("invalid-edit")
            && escalated.detail.contains("iterate local→frontier")
            && escalated
                .detail
                .contains("per pack insurance-verification routing policy"),
        "escalation cites reason and policy: {}",
        escalated.detail
    );
}

#[test]
fn gate_regression_escalates_and_the_local_edit_is_discarded() {
    // The local model tries to unwire phi-encryption — a satisfied required
    // gate. The pack policy names gate-regression, so the edit is discarded
    // and the frontier tier redoes the instruction.
    let regressing = r#"{"message":"dropped encryption for speed","added_feature":"fast mode","wired_controls":[],"removed_controls":["phi-encryption"]}"#;
    let (local_url, _) = mock_model(regressing);
    let (frontier_url, frontier_hits) = mock_model(VALID_EDIT);
    let dispatcher = Dispatcher::new(Some(local_url), Some(frontier_url));
    let pack = pack("insurance-verification");
    let mut app = sandbox_app(&pack);

    let (reply, decisions) = dispatcher.iterate(&mut app, "make lookups faster", &pack);

    assert!(
        app.controls.contains("phi-encryption"),
        "the regressing edit must never be applied"
    );
    assert!(!app.features.iter().any(|f| f == "fast mode"));
    assert_eq!(frontier_hits.load(Ordering::SeqCst), 1);
    assert_eq!(reply.message, "✓ frontier applied the edit");
    let escalated = decisions
        .iter()
        .find(|d| d.action == "agent.escalated")
        .expect("escalation recorded");
    assert!(
        escalated.detail.contains("gate-regression") && escalated.detail.contains("phi-encryption"),
        "escalation names the regressed gate: {}",
        escalated.detail
    );
}

#[test]
fn unlisted_failure_does_not_escalate_it_falls_back_to_rules() {
    // post-op-monitor declares no routing → platform defaults, and the
    // default escalate_on list is empty: an invalid local edit must NOT
    // spend frontier tokens; the deterministic rules floor answers instead.
    let (local_url, local_hits) = mock_model("not an edit");
    let (frontier_url, frontier_hits) = mock_model(VALID_EDIT);
    let dispatcher = Dispatcher::new(Some(local_url), Some(frontier_url));
    let pack = pack("post-op-monitor");
    let mut app = sandbox_app(&pack);

    let (reply, decisions) = dispatcher.iterate(&mut app, "flag anything over 7 to me", &pack);

    assert_eq!(local_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        frontier_hits.load(Ordering::SeqCst),
        0,
        "escalation happens only per the pack's escalate_on list"
    );
    assert!(decisions.iter().all(|d| d.action != "agent.escalated"));
    assert!(
        decisions
            .iter()
            .any(|d| d.detail.contains("not in escalate_on")
                && d.detail.contains("fallback to rules")),
        "fallback is recorded: {decisions:?}"
    );
    // The rules floor still applied the edit — workflow never dies.
    assert!(reply.message.starts_with("✓ done"));
    assert!(app.controls.contains("escalation-path"));
}

// ---------- audit stream, end to end over the API ----------

mod api {
    use axum::body::Body;
    use axum::http::Request;
    use rust_proof_service::api::router_with_dispatcher;
    use rust_proof_service::packs::builtin_packs;
    use rust_proof_service::state::Platform;
    use serde_json::{json, Value};
    use std::sync::{Arc, RwLock};
    use tower::ServiceExt;

    use super::Dispatcher;

    async fn call(router: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> Value {
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
        assert!(res.status().is_success(), "{method} {uri} failed");
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn audit_events_cite_the_policy_source() {
        // No model endpoints: every tier resolves to rules, yet every
        // decision still lands in the audit stream citing its policy.
        let platform = Arc::new(RwLock::new(Platform::new(builtin_packs())));
        let router = router_with_dispatcher(platform, Arc::new(Dispatcher::new(None, None)));

        let created = call(
            &router,
            "POST",
            "/api/apps",
            Some(json!({
                "prompt": "checks each new patient's insurance",
                "pack": "insurance-verification",
                "name": "insurance checker"
            })),
        )
        .await;
        let id = created["app"]["id"].as_str().unwrap().to_string();
        call(
            &router,
            "POST",
            &format!("/api/apps/{id}/iterate"),
            Some(json!({"instruction": "add a payer note field"})),
        )
        .await;
        call(&router, "POST", &format!("/api/apps/{id}/review"), None).await;

        let audit = call(&router, "GET", &format!("/api/apps/{id}/audit"), None).await;
        let routed: Vec<String> = audit["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["action"] == "agent.routed")
            .map(|e| e["detail"].as_str().unwrap().to_string())
            .collect();

        assert!(
            routed
                .iter()
                .any(|d| d
                    .contains("per pack insurance-verification routing policy: scaffold→frontier")),
            "scaffold decision cited: {routed:?}"
        );
        assert!(
            routed.iter().any(|d| {
                d.contains("per pack insurance-verification routing policy: iterate→local")
            }),
            "iterate decision cited: {routed:?}"
        );
        assert!(
            routed
                .iter()
                .any(|d| d
                    .contains("per pack insurance-verification routing policy: review→frontier")),
            "review decision cited: {routed:?}"
        );

        // A defaults pack cites the platform default instead.
        let created = call(
            &router,
            "POST",
            "/api/apps",
            Some(json!({
                "prompt": "post-op recovery tracker",
                "pack": "post-op-monitor",
                "name": "post-op tracker"
            })),
        )
        .await;
        let id2 = created["app"]["id"].as_str().unwrap().to_string();
        let audit = call(&router, "GET", &format!("/api/apps/{id2}/audit"), None).await;
        let details: Vec<&str> = audit["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["action"] == "agent.routed")
            .map(|e| e["detail"].as_str().unwrap())
            .collect();
        assert!(
            details.iter().any(
                |d| d.contains("platform default routing (pack post-op-monitor declares none)")
            ),
            "defaults cited: {details:?}"
        );
    }
}
