//! Contract tests for the describe → audit workflow.
//!
//! The reliability proof this repo promises: the gate is load-bearing. An
//! app with a failing check cannot reach the prod pool, a green report plus
//! a co-signature can, and every transition lands in the append-only audit
//! stream. Exercised end-to-end through the public API, the way any client
//! (doctor UI, CLI, hospital integration) would drive it.

use axum::body::Body;
use axum::http::{Request, StatusCode};

use rust_proof_service::app;
use serde_json::{json, Value};
use tower::ServiceExt;

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

async fn create_post_op_app(router: &axum::Router) -> String {
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

#[tokio::test]
async fn registry_serves_signed_wave_one_packs() {
    let router = app();
    let (status, body) = call(&router, "GET", "/api/packs", None).await;
    assert_eq!(status, StatusCode::OK);
    let packs = body["packs"].as_array().unwrap();
    assert_eq!(packs.len(), 5);
    for pack in packs {
        assert_eq!(pack["signed_by"], "platform-root-v1");
    }
    let wave1: Vec<&str> = packs
        .iter()
        .filter(|p| p["wave"] == 1)
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        wave1.len(),
        3,
        "RFC wave 1: compliance-checklist, hypertension-tracker, patient-intake"
    );
}

#[tokio::test]
async fn describe_lands_in_sandbox_on_synthetic_data() {
    let router = app();
    let id = create_post_op_app(&router).await;
    let (_, app_body) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    assert_eq!(app_body["stage"], "sandbox");
    assert_eq!(app_body["data_source"]["kind"], "synthetic");
    assert!(app_body["features"].as_array().unwrap().len() >= 4);
    // The scaffold pre-wires hipaa-core controls but NOT auto-logoff:
    // the doctor (or "fix it for me") has to close that check.
    let controls = app_body["controls"].as_array().unwrap();
    assert!(controls.iter().any(|c| c == "phi-encryption"));
    assert!(!controls.iter().any(|c| c == "auto-logoff"));
}

#[tokio::test]
async fn gate_blocks_promotion_until_fixed_then_admits_with_cosign() {
    let router = app();
    let id = create_post_op_app(&router).await;

    // Preflight: 5/6, auto-logoff failing and marked fixable (storyboard 1a⑤).
    let (status, gate) = call(&router, "GET", &format!("/api/apps/{id}/gate"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(gate["report"]["passed"], 5);
    assert_eq!(gate["report"]["total"], 6);
    assert_eq!(gate["report"]["green"], false);
    let failing: Vec<&Value> = gate["report"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["status"] == "fail")
        .collect();
    assert_eq!(failing.len(), 1);
    assert_eq!(failing[0]["id"], "auto-logoff");
    assert_eq!(failing[0]["fixable"], true);

    // Deploy is locked: the false-pass guard. The error must name the gap.
    let (status, err) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(err["error"].as_str().unwrap().contains("auto-logoff"));

    // Still sandboxed — the failed promotion changed nothing.
    let (_, app_body) = call(&router, "GET", &format!("/api/apps/{id}"), None).await;
    assert_eq!(app_body["stage"], "sandbox");
    assert!(app_body["allocation"].is_null());

    // "fix it for me", then promote with a co-signature.
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A promotion without a co-signer is still refused.
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "  "})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, promoted) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "promotion should pass: {promoted}");
    assert_eq!(promoted["app"]["stage"], "live");
    assert_eq!(promoted["app"]["allocation"]["pool"], "prod");
    assert_eq!(promoted["app"]["data_source"]["kind"], "tenant");
    assert_eq!(promoted["app"]["attestation"]["cosigner"], "Dr. A. Osei");
    assert_eq!(promoted["app"]["attestation"]["gate_summary"], "6/6");
}

#[tokio::test]
async fn iterate_wires_controls_and_restore_rebuilds_state() {
    let router = app();
    let id = create_post_op_app(&router).await;

    // Addendum 2: a flag rule — the agent wires the escalation path too.
    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "make pain a 0-10 scale and flag anything over 7 to me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["app"]["current_version"], 2);
    assert!(body["reply"]["wired_controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "escalation-path"));

    // Addendum 3, then restore v2: derived state, so the v3 feature vanishes.
    let (_, v3) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "add a wound photo comparison view"})),
    )
    .await;
    let features_v3 = v3["app"]["features"].as_array().unwrap().len();

    let (status, restored) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/restore"),
        Some(json!({"version": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(restored["current_version"], 2);
    assert_eq!(
        restored["features"].as_array().unwrap().len(),
        features_v3 - 1
    );
    assert!(restored["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "escalation-path"));
}

#[tokio::test]
async fn nine_gate_pack_requires_platform_review_before_cosign() {
    let router = app();
    let (_, created) = call(
        &router,
        "POST",
        "/api/apps",
        Some(json!({
            "prompt": "checks each new patient's insurance before their first visit",
            "pack": "insurance-verification",
            "name": "insurance checker"
        })),
    )
    .await;
    let id = created["app"]["id"].as_str().unwrap().to_string();

    let (_, gate) = call(&router, "GET", &format!("/api/apps/{id}/gate"), None).await;
    assert_eq!(
        gate["report"]["total"], 9,
        "storyboard 1b promises nine checks"
    );

    // Wire the fixable staff-safety gates the way the doctor would.
    for g in ["auto-logoff", "access-roles", "escalation-path"] {
        call(
            &router,
            "POST",
            &format!("/api/apps/{id}/gate/{g}/fix"),
            Some(json!({})),
        )
        .await;
    }

    // Review is not auto-fixable: it takes the platform reviewer.
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/human-review/fix"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, review) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/review"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(review["reviewer_note"]
        .as_str()
        .unwrap()
        .contains("Meets release criteria"));

    let (status, promoted) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{promoted}");
    assert_eq!(promoted["app"]["attestation"]["gate_summary"], "9/9");
    assert!(promoted["app"]["attestation"]["reviewer_note"]
        .as_str()
        .unwrap()
        .contains("Meets release criteria"));
}

#[tokio::test]
async fn audit_stream_records_the_whole_story_append_only() {
    let router = app();
    let id = create_post_op_app(&router).await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei"})),
    )
    .await;

    let (_, audit) = call(&router, "GET", &format!("/api/apps/{id}/audit"), None).await;
    let events = audit["events"].as_array().unwrap();
    let actions: Vec<&str> = events
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for expected in [
        "app.created",
        "agent.scaffolded",
        "gate.fixed",
        "gate.passed",
        "app.promoted",
    ] {
        assert!(
            actions.contains(&expected),
            "missing {expected} in {actions:?}"
        );
    }
    // Sequence numbers are strictly increasing — an edited or deleted event
    // would show as a gap or reorder in the export.
    let seqs: Vec<u64> = events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));

    let deploy = events
        .iter()
        .find(|e| e["action"] == "app.promoted")
        .unwrap();
    assert!(deploy["detail"].as_str().unwrap().contains("preflight 6/6"));
    assert!(deploy["detail"]
        .as_str()
        .unwrap()
        .contains("co-signed Dr. A. Osei"));
}

#[tokio::test]
async fn export_renders_nomad_job_pinned_to_prod_pool() {
    let router = app();
    let id = create_post_op_app(&router).await;
    // No allocation yet → the bundle still ships (no hostage docs), but the
    // compliance record is a draft with no attestation and a stub Nomad job.
    let (status, draft) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    assert_eq!(status, StatusCode::OK);
    let compliance = draft["files"]["docs/COMPLIANCE.md"].as_str().unwrap();
    assert!(compliance.contains("draft — not released"));
    assert!(!compliance.contains("co-signed by:"), "no attestation yet");
    assert!(draft["files"]["nomad/job.nomad.hcl"]
        .as_str()
        .unwrap()
        .contains("no live allocation yet"));

    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei"})),
    )
    .await;

    let (status, export) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    assert_eq!(status, StatusCode::OK);
    let job = export["files"]["nomad/job.nomad.hcl"].as_str().unwrap();
    assert!(job.contains(&format!("job \"{id}\"")));
    assert!(
        job.contains("value     = \"prod\""),
        "job must constrain to the prod pool"
    );
    assert!(job.contains("driver = \"docker\""));
    assert!(job.contains("namespace   = \"tenant-meridian\""));
    assert!(!job.contains("{{app_id}}"), "no unrendered tokens");
}

/// GOAL.md bars 5 and 6: eject produces a repo a stranger can run from the
/// included docs alone, and the app becomes the doctor's own template.
#[tokio::test]
async fn ejection_bundle_carries_the_doctors_record_and_a_reimportable_pack() {
    let router = app();
    let id = create_post_op_app(&router).await;

    // Iterate once, fix the failing gate, promote with a co-signature.
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(json!({"instruction": "make pain a 0-10 scale and flag anything over 7 to me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
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
        Some(json!({"cosigner": "Dr. A. Osei"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, export) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    assert_eq!(status, StatusCode::OK);

    let files = export["files"].as_object().unwrap();
    for path in [
        "README.md",
        "docs/RUNBOOK.md",
        "docs/COMPLIANCE.md",
        "Dockerfile",
        "render.yaml",
        "fly.toml",
        "config/deploy.yml",
        "nomad/job.nomad.hcl",
        "pack.hcl",
    ] {
        assert!(files.contains_key(path), "bundle is missing {path}");
    }
    assert!(
        export["unpack"].as_str().unwrap().contains("python3"),
        "response carries a copy-paste unpack one-liner"
    );

    // README is the doctor's story: their prompt and each addendum.
    let readme = files["README.md"].as_str().unwrap();
    assert!(readme.contains("a post-op recovery tracker for my knee replacement patients"));
    assert!(readme.contains("make pain a 0-10 scale and flag anything over 7 to me"));

    // COMPLIANCE embeds the release: re-run gate report, cosigner, audit.
    let compliance = files["docs/COMPLIANCE.md"].as_str().unwrap();
    assert!(compliance.contains("6/6"));
    assert!(compliance.contains("Dr. A. Osei"));
    assert!(compliance.contains("app.promoted"), "audit trail embedded");

    // RUNBOOK says plainly that the app source is a scaffold placeholder (#5).
    let runbook = files["docs/RUNBOOK.md"].as_str().unwrap();
    assert!(runbook.contains("scaffold placeholder"));

    // pack.hcl parses with the platform's own parser: their own template.
    let pack_hcl = files["pack.hcl"].as_str().unwrap();
    let template = rust_proof_service::packs::parse_pack(pack_hcl)
        .expect("ejected pack.hcl must round-trip through packs::parse_pack");
    assert_eq!(template.id, format!("{id}-template"));
    assert!(template
        .scaffold
        .iter()
        .any(|f| f.contains("make pain a 0-10 scale")));
}

#[tokio::test]
async fn rollback_destroys_allocation_and_returns_to_synthetic_data() {
    let router = app();
    let id = create_post_op_app(&router).await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei"})),
    )
    .await;

    let (status, back) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/rollback"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(back["stage"], "sandbox");
    assert!(back["allocation"].is_null());
    assert_eq!(back["data_source"]["kind"], "synthetic");
}

#[tokio::test]
async fn doctor_ui_is_served_but_holds_no_privileges() {
    let router = app();
    let res = router
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let html = String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        html.contains("/api/apps"),
        "the UI is a client of the same API"
    );
}
