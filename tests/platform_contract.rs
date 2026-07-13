//! Contract tests for the describe → audit workflow.
//!
//! The reliability proof this repo promises: the gate is load-bearing. An
//! app with a failing check cannot reach the prod pool, a green report plus
//! a co-signature can, and every transition lands in the append-only audit
//! stream. Exercised end-to-end through the public API, the way any client
//! (doctor UI, CLI, hospital integration) would drive it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::fs;
use std::path::Path;

use rust_proof_service::app;
use serde_json::{json, Value};
use tower::ServiceExt;

#[test]
fn production_configuration_has_one_application_model_boundary() {
    let mut active_configuration = [
        include_str!("../env.example"),
        include_str!("../docker-compose.yml"),
        include_str!("../scripts/staging-up.sh"),
        include_str!("../docs/rfc/0001-clinician-platform.md"),
        include_str!("../src/ladder.rs"),
        include_str!("../src/state.rs"),
    ]
    .join("\n");
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
    for entry in fs::read_dir(scripts).expect("read scripts") {
        let path = entry.expect("read script entry").path();
        if path.is_file() {
            active_configuration.push_str(
                &fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
            );
        }
    }
    for retired in [
        "LOCAL_MODEL_URL",
        "FRONTIER_MODEL_URL",
        "MODEL_URL=",
        "MODEL_HTTP_TIMEOUT_SECS",
        "OPENAI_API_KEY",
        "gpt-",
        "Hermes",
        "Liquid",
        "Open SWE",
        "Deep Agents",
        "SmolLM",
        "llama.cpp",
        "llama_cpp.server",
    ] {
        assert!(
            !active_configuration.contains(retired),
            "active configuration still exposes retired model path {retired}"
        );
    }

    let workspace_agent = include_str!("../src/workspace_agent.rs");
    assert!(workspace_agent.contains("gemma-4-31B-it"));
    assert!(workspace_agent.contains("unsupported WORKSPACE_AGENT_PROVIDER"));
}

#[test]
fn treatment_choice_never_interpolates_model_data_into_javascript() {
    let ui = include_str!("../web/index.html");
    assert!(ui.contains("onchange=\"selectTreatment(this.value)\""));
    assert!(!ui.contains("selectTreatment('${esc(treatment.id)}')"));
    assert!(
        ui.contains("Rust prepared a model-free fallback treatment because Gemma was unavailable.")
    );
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
    assert_eq!(packs.len(), 17);
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
        5,
        "wave 1 includes checklist, hypertension, intake, portal, and dashboard"
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
async fn source_treatment_is_reviewed_before_it_changes_the_export() {
    let router = app();
    let id = create_post_op_app(&router).await;
    let workspace_url = format!("/api/apps/{id}/workspace");

    let (_, initial) = call(&router, "GET", &workspace_url, None).await;
    let initial_digest = initial["accepted"]["digest"].as_str().unwrap().to_string();
    assert_eq!(initial["accepted"]["version"], 0);

    let (status, planned) = call(
        &router,
        "POST",
        &format!("{workspace_url}/treatments"),
        Some(json!({"task":"make follow-up work easier to scan"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{planned}");
    assert_eq!(planned["phase"], "treatments_ready");
    assert_eq!(planned["plan_agent"]["provider"], "deterministic");
    assert_eq!(planned["plan_agent"]["model"], "convention-floor-v1");
    assert!(planned["plan_agent"].get("fallback_reason").is_none());
    assert_eq!(
        planned["treatment_plan"]["treatments"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let (status, invalid_refinement) = call(
        &router,
        "POST",
        &format!("{workspace_url}/select"),
        Some(json!({
            "treatment_id":"event-timeline",
            "refinement": {"presentation":"context-first", "emphasis":"x".repeat(501)}
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{invalid_refinement}"
    );
    let (_, unchanged) = call(&router, "GET", &workspace_url, None).await;
    assert!(unchanged["selected_treatment"].is_null());

    let (status, selected) = call(
        &router,
        "POST",
        &format!("{workspace_url}/select"),
        Some(json!({
            "treatment_id":"event-timeline",
            "refinement": {
                "presentation": "context-first",
                "emphasis": "Show exactly why the practice inbox was notified."
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{selected}");
    assert_eq!(
        selected["selected_treatment"]["treatment"],
        planned["treatment_plan"]["treatments"][1]
    );
    assert_eq!(
        selected["selected_treatment"]["refinement"]["presentation"],
        "context-first"
    );
    assert_eq!(
        selected["selected_treatment"]["planner"],
        planned["plan_agent"]
    );

    let (status, review) = call(
        &router,
        "POST",
        &format!("{workspace_url}/generate"),
        Some(json!({"task":"show unresolved events first"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{review}");
    assert_eq!(review["phase"], "review_required");
    assert_eq!(review["generation_agent"]["provider"], "rust");
    assert_eq!(review["generation_agent"]["model"], "rust-convention-v2");
    assert_eq!(review["accepted"]["digest"], initial_digest);
    assert_eq!(review["candidate"]["verification"]["passed"], true);
    assert_eq!(
        review["candidate"]["diff"][0]["path"],
        "web/src/lib/treatment.json"
    );
    let candidate_config: Value =
        serde_json::from_str(review["candidate"]["files"][0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(candidate_config["treatment"]["id"], "event-timeline");
    assert_eq!(
        candidate_config["refinement"]["presentation"],
        "context-first"
    );
    assert_eq!(
        candidate_config["refinement"]["emphasis"],
        "Show exactly why the practice inbox was notified."
    );
    let rejected_id = review["candidate"]["id"].as_str().unwrap();

    let (_, rejected) = call(
        &router,
        "POST",
        &format!("{workspace_url}/candidate/reject"),
        Some(json!({"candidate_id":rejected_id})),
    )
    .await;
    assert_eq!(rejected["accepted"]["digest"], initial_digest);

    let (_, review) = call(
        &router,
        "POST",
        &format!("{workspace_url}/generate"),
        Some(json!({"task":"show unresolved events first"})),
    )
    .await;
    let candidate_id = review["candidate"]["id"].as_str().unwrap();
    let (status, accepted) = call(
        &router,
        "POST",
        &format!("{workspace_url}/candidate/accept"),
        Some(json!({"candidate_id":candidate_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["accepted"]["version"], 1);
    assert_ne!(accepted["accepted"]["digest"], initial_digest);

    let (_, export) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    let exported_config: Value = serde_json::from_str(
        export["files"]["web/src/lib/treatment.json"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(exported_config, candidate_config);
    assert!(export["files"]["web/src/lib/TreatmentWorkspace.svelte"]
        .as_str()
        .unwrap()
        .contains("recipe.id === 'event-timeline'"));
    assert!(export["files"]["web/src/routes/+page.svelte"]
        .as_str()
        .unwrap()
        .contains("<TreatmentWorkspace"));
    assert!(export["files"]["web/src/lib/PostOpCheckIn.svelte"]
        .as_str()
        .unwrap()
        .contains("treatment.refinement.presentation === 'context-first'"));
}

#[tokio::test]
async fn gate_blocks_promotion_until_fixed_then_admits_with_cosign() {
    let router = app();
    let id = create_post_op_app(&router).await;

    // Preflight: auto-logoff failing and marked fixable (storyboard 1a⑤).
    // Evidence basis (#3): four verdicts are inspected from the pack's
    // scaffold source; the encryption stub reports `stubbed`, never `pass`,
    // so `passed` counts 4 with 1 stubbed alongside.
    let (status, gate) = call(&router, "GET", &format!("/api/apps/{id}/gate"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(gate["report"]["passed"], 4);
    assert_eq!(gate["report"]["stubbed"], 1);
    assert_eq!(gate["report"]["total"], 6);
    assert_eq!(gate["report"]["green"], false);
    let audit_gate = gate["report"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "audit-log")
        .unwrap();
    assert_eq!(audit_gate["basis"], "evidence");
    assert_eq!(audit_gate["citation"], "45 CFR §164.312(b)");
    let phi = gate["report"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "phi-encryption")
        .unwrap();
    assert_eq!(phi["status"], "stubbed", "a stub must never read as pass");
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
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
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
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(promoted["error"]
        .as_str()
        .unwrap()
        .contains("phi-encryption"));

    let (_, audit) = call(&router, "GET", &format!("/api/apps/{id}/audit"), None).await;
    let denial = audit["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| {
            event["action"] == "gate.promotion_denied"
                && event["detail"]
                    .as_str()
                    .is_some_and(|detail| detail.contains("STUBBED"))
        })
        .expect("denied real-data promotion is audited");
    assert_eq!(denial["actor"], "dr-osei");
    assert!(denial["detail"].as_str().unwrap().contains("STUBBED"));
    assert!(denial["detail"].as_str().unwrap().contains(&id));

    let (status, promoted) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "promotion should pass: {promoted}");
    assert_eq!(promoted["app"]["stage"], "live");
    assert_eq!(promoted["app"]["allocation"]["pool"], "synthetic-demo");
    assert_eq!(promoted["app"]["data_source"]["kind"], "synthetic");
    assert_eq!(promoted["app"]["attestation"]["cosigner"], "Dr. A. Osei");
    assert_eq!(
        promoted["app"]["attestation"]["gate_summary"], "5/6 (1 stubbed)",
        "the attestation discloses the stub instead of absorbing it"
    );
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
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
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
    assert!(deploy["detail"]
        .as_str()
        .unwrap()
        .contains("preflight 5/6 (1 stubbed)"));
    assert!(deploy["detail"]
        .as_str()
        .unwrap()
        .contains("co-signed Dr. A. Osei"));
}

#[tokio::test]
async fn synthetic_demo_export_does_not_claim_a_prod_nomad_job() {
    let router = app();
    let id = create_post_op_app(&router).await;
    // No allocation yet → the bundle still ships (no hostage docs), but the
    // compliance record is a draft with no attestation and a stub Nomad job.
    let (status, draft) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    assert_eq!(status, StatusCode::OK);
    let compliance = draft["files"]["README.md"].as_str().unwrap();
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
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;

    let (status, export) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    assert_eq!(status, StatusCode::OK);
    let job = export["files"]["nomad/job.nomad.hcl"].as_str().unwrap();
    assert!(job.contains("value     = \"synthetic-demo\""));
    assert!(!job.contains("value     = \"prod\""));
    assert!(
        !job.contains("vault {"),
        "synthetic demos receive no tenant credentials"
    );
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
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, export) = call(&router, "GET", &format!("/api/apps/{id}/export"), None).await;
    assert_eq!(status, StatusCode::OK);

    let files = export["files"].as_object().unwrap();
    for path in [
        "README.md",
        "web/package-lock.json",
        "web/src/routes/+page.svelte",
        "server/src/main.rs",
        "diagrams/system-architecture.tldr",
        "diagrams/workspace-state-machine.tldr",
        "diagrams/service-map.tldr",
        "Dockerfile",
        "config/nginx.conf",
        "config/start.sh",
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

    // COMPLIANCE embeds the release: the attestation-time gate report
    // frozen at promotion (F3), cosigner, audit — stub disclosed, cited.
    let compliance = files["README.md"].as_str().unwrap();
    assert!(compliance.contains("5/6 (1 stubbed)"));
    assert!(compliance.contains("frozen at promotion"));
    assert!(compliance.contains("STUBBED —"), "no false passes");
    assert!(compliance.contains("45 CFR §164.312(b)"), "P1 citations");
    assert!(compliance.contains("Dr. A. Osei"));
    assert!(compliance.contains("app.promoted"), "audit trail embedded");

    // post-op-monitor is converted to the runnable-scaffold spec (#5): the
    // bundle carries the real app source and the runbook drops the
    // placeholder caveat it used to need.
    assert!(
        files.contains_key("server/src/main.rs"),
        "real app source ships"
    );
    assert!(files.contains_key("server/Cargo.toml"));
    assert!(
        files["synthetic/post-op-demo.json"]
            .as_str()
            .unwrap()
            .contains("SYNTHETIC DATA"),
        "the synthetic seed travels with the app"
    );
    let runbook = files["README.md"].as_str().unwrap();
    assert!(!runbook.contains("scaffold placeholder"), "{runbook}");
    assert!(runbook.contains("The app source is real"));

    // The owned manifest parses as untrusted metadata and is refused by the
    // trusted built-in registry parser.
    let pack_hcl = files["pack.hcl"].as_str().unwrap();
    assert!(rust_proof_service::packs::parse_pack(pack_hcl).is_err());
    let template = rust_proof_service::packs::parse_owned_pack(pack_hcl)
        .expect("ejected pack.hcl must parse as owned metadata");
    assert_eq!(template.id, format!("{id}-template"));
    assert!(template
        .scaffold
        .iter()
        .any(|f| f.contains("make pain a 0-10 scale")));
}

#[tokio::test]
async fn owned_bundle_reimport_preserves_customized_source_without_inheriting_authority() {
    let router = app();
    let original_id = create_post_op_app(&router).await;
    let (status, mut exported) = call(
        &router,
        "GET",
        &format!("/api/apps/{original_id}/export"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let files = exported["files"].as_object_mut().unwrap();
    for (path, sentinel) in [
        ("server/src/main.rs", "// imported-server-sentinel"),
        (
            "web/src/routes/+page.svelte",
            "<!-- imported-svelte-sentinel -->",
        ),
        ("web/tests/owned-app.mjs", "// imported-browser-sentinel"),
        ("README.md", "Owned customization record."),
    ] {
        let changed = format!("{}\n{sentinel}\n", files[path].as_str().unwrap());
        files.insert(path.into(), Value::String(changed));
    }
    for path in ["synthetic/post-op-demo.json", "artifact-quality.json"] {
        let changed = format!("{}\n", files[path].as_str().unwrap());
        files.insert(path.into(), Value::String(changed));
    }
    let owned_manifest = files["pack.hcl"].as_str().unwrap().replacen(
        "prewired = [",
        "prewired = [\n    \"rogue-control\",",
        1,
    );
    files.insert("pack.hcl".into(), Value::String(owned_manifest));
    let expected = files.clone();

    let (status, imported) = call(
        &router,
        "POST",
        "/api/apps/import",
        Some(json!({"files": expected})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "import failed: {imported}");
    let imported_id = imported["app"]["id"].as_str().unwrap();
    assert_ne!(imported_id, original_id);
    assert_eq!(imported["app"]["stage"], "sandbox");
    assert_eq!(imported["app"]["data_source"]["kind"], "synthetic");
    assert!(imported["app"]["allocation"].is_null());
    assert!(imported["app"]["attestation"].is_null());
    assert!(!imported["app"]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| control == "rogue-control"));
    assert!(imported["verification"]["passed"].as_bool().unwrap());
    assert_eq!(
        imported["source_digest"],
        imported["verification"]["workspace_digest"]
    );

    let (status, workspace) = call(
        &router,
        "GET",
        &format!("/api/apps/{imported_id}/workspace"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(workspace["origin"], "owned_import_verified");
    assert_eq!(workspace["accepted"]["files"], json!(expected));

    let (status, reexported) = call(
        &router,
        "GET",
        &format!("/api/apps/{imported_id}/export"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for path in [
        "server/src/main.rs",
        "web/src/routes/+page.svelte",
        "web/tests/owned-app.mjs",
        "synthetic/post-op-demo.json",
        "artifact-quality.json",
        "README.md",
    ] {
        assert_eq!(reexported["files"][path], expected[path], "changed {path}");
    }

    let (status, packs) = call(&router, "GET", "/api/packs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(packs["packs"].as_array().unwrap().len(), 17);
    let (status, denied) = call(
        &router,
        "POST",
        &format!("/api/apps/{imported_id}/promote"),
        Some(json!({"cosigner":"Dr. A. Osei"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(denied["error"]
        .as_str()
        .unwrap()
        .contains("synthetic demo pool"));

    let mut forged = expected;
    let manifest = forged["pack.hcl"]
        .as_str()
        .unwrap()
        .replace("untrusted-practice-export", "platform-root-v1");
    forged.insert("pack.hcl".into(), Value::String(manifest));
    let (status, rejected) = call(
        &router,
        "POST",
        "/api/apps/import",
        Some(json!({"files": forged})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(rejected["error"]
        .as_str()
        .unwrap()
        .contains("untrusted-practice-export"));
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
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
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
    assert!(html.contains("/* Project-owned warm clinician theme."));
    assert!(
        html.contains("min-height:44px"),
        "touch targets stay at least 44px"
    );
    assert!(
        html.contains("prefers-reduced-motion:reduce"),
        "motion-sensitive clinicians can disable nonessential movement"
    );
    assert!(
        !html.contains("_ds_bundle") && !html.contains("catalyst-ui-kit"),
        "the restricted vendor kit never ships in the studio"
    );
}

/// #6 (honest slice): operate reports Nomad's dual status axes. In
/// simulated mode (no NOMAD_ADDR — every test) the observed axis mirrors
/// the desired one and SAYS SO via `status_source: "simulated"` — labeled,
/// never claimed. The staging pressure test asserts the real-Nomad side
/// (`status_source: "nomad"`, observed matching Nomad's own word).
#[tokio::test]
async fn operate_reports_dual_status_axes_labeled_simulated() {
    let router = app();
    let id = create_post_op_app(&router).await;

    // Sandbox: nothing is desired to run, and nothing is observed running.
    let (status, operate) = call(&router, "GET", &format!("/api/apps/{id}/operate"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(operate["desired_state"], "stopped");
    assert_eq!(operate["observed_state"], "stopped");
    assert_eq!(operate["status_source"], "simulated");

    // Promote, then the record claims running — and simulated mode mirrors
    // it on the observed axis rather than inventing an observation.
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(json!({})),
    )
    .await;
    let (status, promoted) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{promoted}");

    let (_, operate) = call(&router, "GET", &format!("/api/apps/{id}/operate"), None).await;
    assert_eq!(operate["desired_state"], "running");
    assert_eq!(operate["observed_state"], "running");
    assert_eq!(operate["status_source"], "simulated");
    assert_eq!(operate["metrics"]["available"], false);
    assert!(operate["metrics"]["uptime_pct"].is_null());
    assert!(operate["metrics"]["p95_ms"].is_null());
    assert_eq!(
        operate["metrics"]["healthy"], false,
        "simulated status must never be promoted into an observed health claim"
    );
}
