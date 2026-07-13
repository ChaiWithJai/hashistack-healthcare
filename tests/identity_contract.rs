//! Contract tests for identity, tenancy, roles, and the authenticated
//! co-sign record (#10), exercised end-to-end through the public API the way any
//! client would drive it.
//!
//! The bar (issue #10): two tenants on one control plane — cross-tenant
//! reads and promotes answer 404 (existence undisclosed) and land in the
//! audit stream; staff cannot promote/co-sign or export the platform audit
//! (403, audited); the attestation binds the authenticated principal + a
//! sha256 digest of the frozen gate report + a timestamp; the dev fallback
//! keeps the zero-config UI working AND confesses itself in the audit trail.

use axum::body::Body;
use axum::http::{Request, StatusCode};

use rust_proof_service::api;
use rust_proof_service::gates;
use rust_proof_service::identity::Registry;
use rust_proof_service::packs;
use rust_proof_service::state::Platform;
use serde_json::{json, Value};
use sha2::Digest;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

const OSEI: &str = "dev-token-osei"; // clinician, meridian
const PARK: &str = "dev-token-park"; // clinician, lakeside
const STAFF: &str = "dev-token-rivera"; // staff, meridian

/// The dev-mode router: embedded registry, dr-osei fallback, no idle.
fn dev_router() -> axum::Router {
    api::router()
}

/// A strict-mode router (as staging boots): same principals, no fallback,
/// optional session idle.
fn strict_router(idle_secs: Option<u64>) -> axum::Router {
    let mut platform = Platform::new(packs::builtin_packs());
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/staging/identities.hcl"
    ))
    .expect("staging/identities.hcl readable");
    platform.identity = Arc::new(Registry::parse(&source, None, idle_secs).unwrap());
    api::router_with_state(Arc::new(RwLock::new(platform)))
}

async fn call(
    router: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let req = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let raw = String::from_utf8_lossy(&bytes).to_string();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value, raw)
}

async fn create_app(router: &axum::Router, token: &str, pack: &str, name: &str) -> String {
    let (status, body, _) = call(
        router,
        "POST",
        "/api/apps",
        Some(token),
        Some(json!({"prompt": format!("{name} for my patients"), "pack": pack, "name": name})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create failed: {body}");
    body["app"]["id"].as_str().unwrap().to_string()
}

async fn guest_cookie(router: &axum::Router) -> String {
    let req = Request::builder()
        .method("POST")
        .uri("/api/public/session")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    res.headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

async fn guest_call(
    router: &axum::Router,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    let req = match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
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

#[tokio::test]
async fn anonymous_workspaces_are_private_synthetic_and_export_requires_an_owner() {
    let router = strict_router(None);
    let first = guest_cookie(&router).await;
    let second = guest_cookie(&router).await;
    assert_ne!(first, second);
    assert!(
        !first.contains("anon-"),
        "the derived tenant must not leak in the cookie"
    );

    let (status, created) = guest_call(
        &router,
        "POST",
        "/api/apps",
        &first,
        Some(json!({"prompt":"a synthetic follow-up helper","pack":"outbound-followup"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert!(created["app"]["tenant"]
        .as_str()
        .unwrap()
        .starts_with("anon-"));
    assert!(created["app"]["data_source"]
        .to_string()
        .to_lowercase()
        .contains("synthetic"));
    let id = created["app"]["id"].as_str().unwrap();

    let (status, other) =
        guest_call(&router, "GET", &format!("/api/apps/{id}"), &second, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{other}");

    let (status, _) = guest_call(
        &router,
        "GET",
        &format!("/api/apps/{id}/export"),
        &first,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn guest_completes_the_core_flow_and_auth_is_required_only_at_eject() {
    let router = strict_router(None);
    let cookie = guest_cookie(&router).await;
    let (_, created) = guest_call(
        &router,
        "POST",
        "/api/apps",
        &cookie,
        Some(json!({"prompt":"a synthetic compliance helper","pack":"compliance-checklist"})),
    )
    .await;
    let id = created["app"]["id"].as_str().unwrap();
    let (status, iterated) = guest_call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        &cookie,
        Some(json!({"instruction":"add a calm confirmation screen"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{iterated}");
    let (status, _) = guest_call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        &cookie,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, preview) = guest_call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        &cookie,
        Some(json!({"synthetic_demo":true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["app"]["allocation"]["pool"], "synthetic-demo");

    let claim = Request::builder()
        .method("POST")
        .uri(format!("/api/apps/{id}/claim"))
        .header("authorization", format!("Bearer {OSEI}"))
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let claimed = router.clone().oneshot(claim).await.unwrap();
    assert_eq!(claimed.status(), StatusCode::OK);

    let (status, bundle, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{id}/export"),
        Some(OSEI),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bundle}");
    assert!(bundle["files"]["server/src/main.rs"].is_string());
    assert!(bundle["files"]["web/src/routes/+page.svelte"].is_string());
}

// ---------- dev fallback: zero-config UI keeps working, audited ----------

#[tokio::test]
async fn missing_header_falls_back_to_dr_osei_in_dev_and_confesses_once() {
    let router = dev_router();
    // Two headerless requests — the demo UI's shape.
    let (status, body, _) = call(&router, "GET", "/api/apps", None, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the zero-config UI must keep working"
    );
    assert!(body["apps"].is_array());
    call(&router, "GET", "/api/packs", None, None).await;

    // The trail confesses the convenience — exactly once per boot.
    let (_, _, export) = call(&router, "GET", "/api/audit/export", None, None).await;
    assert_eq!(
        export.matches("\"auth.dev_fallback\"").count(),
        1,
        "first-use-per-boot confession: {export}"
    );
    assert!(export.contains("\"actor\":\"dr-osei\""));

    // A PRESENT but unknown token is still 401 even in dev — the fallback
    // covers a missing header, never a wrong credential.
    let (status, err, _) = call(&router, "GET", "/api/apps", Some("wrong-token"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{err}");
}

#[tokio::test]
async fn open_routes_stay_open_without_credentials() {
    let router = strict_router(None);
    for (method, uri) in [("GET", "/"), ("GET", "/health"), ("POST", "/proof/etl")] {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "{method} {uri} must stay open"
        );
    }
}

// ---------- strict mode (IDENTITIES_FILE): 401s, no fallback ----------

#[tokio::test]
async fn strict_mode_answers_401_for_missing_and_invalid_tokens() {
    let router = strict_router(None);
    let (status, err, _) = call(&router, "GET", "/api/apps", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{err}");
    let (status, _, _) = call(
        &router,
        "POST",
        "/api/apps/import",
        None,
        Some(json!({"files": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(&router, "GET", "/api/apps", Some("nope"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(&router, "GET", "/api/apps", Some(OSEI), None).await;
    assert_eq!(status, StatusCode::OK, "a declared token authenticates");
}

// ---------- tenancy: 404 across the boundary, audited ----------

#[tokio::test]
async fn cross_tenant_access_answers_404_and_lands_in_the_audit_stream() {
    let router = dev_router();
    let park_app = create_app(&router, PARK, "hypertension-tracker", "lakeside bp log").await;

    // The owner reads it fine; the other tenant's clinician gets exactly
    // what a nonexistent id gets — existence is not disclosed.
    let (status, body, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{park_app}"),
        Some(PARK),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["tenant"], "lakeside",
        "tenant comes from the principal"
    );

    for (method, uri, req_body) in [
        ("GET", format!("/api/apps/{park_app}"), None),
        ("GET", format!("/api/apps/{park_app}/gate"), None),
        ("GET", format!("/api/apps/{park_app}/audit"), None),
        ("GET", format!("/api/apps/{park_app}/export"), None),
        (
            "POST",
            format!("/api/apps/{park_app}/promote"),
            Some(json!({"cosigner": "Dr. A. Osei"})),
        ),
    ] {
        let (status, err, _) = call(&router, method, &uri, Some(OSEI), req_body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {err}");
        assert_eq!(err["error"], "app not found", "same body as nonexistent");
    }

    // Lists are tenant-scoped in both directions.
    let (_, osei_list, _) = call(&router, "GET", "/api/apps", Some(OSEI), None).await;
    assert!(
        !osei_list["apps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == park_app.as_str()),
        "meridian's list must not show lakeside apps"
    );
    let (_, park_list, _) = call(&router, "GET", "/api/apps", Some(PARK), None).await;
    assert!(park_list["apps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == park_app.as_str()));

    // Every denial is on the record, actor = the denied principal, on the
    // owning tenant's app stream.
    let (_, _, export) = call(&router, "GET", "/api/audit/export", Some(OSEI), None).await;
    assert!(export.contains("\"auth.cross_tenant_denied\""), "{export}");
    assert!(export.contains("\"actor\":\"dr-osei\""));
    let (_, park_audit, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{park_app}/audit"),
        Some(PARK),
        None,
    )
    .await;
    assert!(
        park_audit["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "auth.cross_tenant_denied" && e["actor"] == "dr-osei"),
        "the owning tenant sees who knocked: {park_audit}"
    );
}

#[tokio::test]
async fn owned_bundle_import_belongs_only_to_the_importing_practice() {
    let router = dev_router();
    let source_id = create_app(&router, OSEI, "post-op-monitor", "meridian starter").await;
    let (status, bundle, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{source_id}/export"),
        Some(OSEI),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, imported, _) = call(
        &router,
        "POST",
        "/api/apps/import",
        Some(PARK),
        Some(json!({"files": bundle["files"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{imported}");
    assert_eq!(imported["app"]["tenant"], "lakeside");
    let imported_id = imported["app"]["id"].as_str().unwrap();

    let (status, _, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{imported_id}"),
        Some(PARK),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, hidden, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{imported_id}"),
        Some(OSEI),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(hidden["error"], "app not found");
}

#[tokio::test]
async fn create_ignores_nothing_a_mismatched_tenant_field_is_refused() {
    let router = dev_router();
    // Matching the principal's tenant is fine (old clients keep working)…
    let (status, body, _) = call(
        &router,
        "POST",
        "/api/apps",
        Some(OSEI),
        Some(json!({"prompt": "x", "pack": "patient-intake", "tenant": "meridian"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["app"]["tenant"], "meridian");
    // …but naming any other tenant is refused loudly, never overridden.
    let (status, err, _) = call(
        &router,
        "POST",
        "/api/apps",
        Some(OSEI),
        Some(json!({"prompt": "x", "pack": "patient-intake", "tenant": "lakeside"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert!(err["error"]
        .as_str()
        .unwrap()
        .contains("derived from the authenticated principal"));
}

// ---------- roles: one capability check, 403s audited ----------

#[tokio::test]
async fn staff_cannot_promote_cosign_or_export_the_platform_audit() {
    let router = dev_router();
    let id = create_app(&router, STAFF, "post-op-monitor", "staff-built tracker").await;
    // Staff work in their tenant: build, fix, review all fine.
    let (status, _, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(STAFF),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // …but the release is a clinical act: 403, audited.
    let (status, err, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(STAFF),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{err}");
    assert!(err["error"].as_str().unwrap().contains("staff"));
    let (status, still, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{id}"),
        Some(STAFF),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(still["stage"], "sandbox", "the refusal changed nothing");

    // The platform-wide export is likewise clinician-only.
    let (status, _, _) = call(&router, "GET", "/api/audit/export", Some(STAFF), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (_, _, export) = call(&router, "GET", "/api/audit/export", Some(OSEI), None).await;
    assert!(export.contains("\"auth.role_denied\""), "{export}");
    assert!(export.contains("\"actor\":\"ms-rivera\""));

    // The same-tenant clinician releases it (their own act, field omitted).
    let (status, promoted, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(OSEI),
        Some(json!({"synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{promoted}");
    assert_eq!(promoted["app"]["attestation"]["principal"], "dr-osei");
}

// ---------- the cryptographic co-sign ----------

#[tokio::test]
async fn attestation_binds_principal_name_and_report_digest_and_the_digest_verifies() {
    let router = dev_router();
    let id = create_app(&router, OSEI, "post-op-monitor", "digest tracker").await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(OSEI),
        Some(json!({})),
    )
    .await;

    // A typed cosigner naming anyone but the authenticated principal is
    // refused — the co-sign record is the principal's own authenticated act.
    let (status, err, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(OSEI),
        Some(json!({"cosigner": "Dr. Somebody Else", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert!(err["error"].as_str().unwrap().contains("co-signature"));

    // Matching the registered name (what the UI types) works…
    let (status, promoted, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(OSEI),
        Some(json!({"cosigner": "Dr. A. Osei", "synthetic_demo": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{promoted}");
    let att = &promoted["app"]["attestation"];
    assert_eq!(att["cosigner"], "Dr. A. Osei");
    assert_eq!(att["principal"], "dr-osei");
    assert!(att["at"].as_u64().unwrap() > 0);

    // …and the digest is sha256 over the frozen report's canonical JSON —
    // recomputed here from the attestation's own frozen report.
    let digest = att["report_digest"].as_str().unwrap();
    assert!(digest.starts_with("sha256:"), "{digest}");
    let frozen: gates::GateReport = serde_json::from_value(att["report"].clone()).unwrap();
    assert_eq!(
        digest,
        gates::report_digest(&frozen),
        "the digest must verify against the frozen report"
    );
    let manual = sha2::Sha256::digest(serde_json::to_string(&frozen).unwrap().as_bytes());
    let manual_hex: String = manual.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        digest,
        format!("sha256:{manual_hex}"),
        "and it is plain sha256"
    );

    // The ejected compliance record renders the whole act.
    let (_, export, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{id}/export"),
        Some(OSEI),
        None,
    )
    .await;
    let compliance = export["files"]["README.md"].as_str().unwrap();
    assert!(
        compliance.contains("(authenticated principal `dr-osei`)"),
        "{compliance}"
    );
    assert!(compliance.contains(digest));
}

// ---------- session idle: the platform honors its own auto-logoff gate ----

#[tokio::test]
async fn idle_session_expires_with_401_and_an_audit_event() {
    let router = strict_router(Some(1));
    let (status, _, _) = call(&router, "GET", "/api/apps", Some(OSEI), None).await;
    assert_eq!(status, StatusCode::OK, "fresh session");

    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    let (status, err, _) = call(&router, "GET", "/api/apps", Some(OSEI), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{err}");
    assert!(err["error"].as_str().unwrap().contains("session expired"));

    // The 401 was the logoff boundary: the next request re-authenticates,
    // and the expiry is on the record with the principal as actor.
    let (status, _, export) = call(&router, "GET", "/api/audit/export", Some(OSEI), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(export.contains("\"auth.session_expired\""), "{export}");
    assert!(export.contains("\"actor\":\"dr-osei\""));
}

// ---------- audit attribution: real principal ids everywhere ----------

#[tokio::test]
async fn audit_actor_is_the_real_principal_id_for_every_doctor_action() {
    let router = dev_router();
    let id = create_app(&router, PARK, "post-op-monitor", "park tracker").await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/gate/auto-logoff/fix"),
        Some(PARK),
        Some(json!({})),
    )
    .await;
    call(
        &router,
        "POST",
        &format!("/api/apps/{id}/promote"),
        Some(PARK),
        Some(json!({"synthetic_demo": true})),
    )
    .await;
    let (_, audit, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{id}/audit"),
        Some(PARK),
        None,
    )
    .await;
    let events = audit["events"].as_array().unwrap();
    let created = events
        .iter()
        .find(|e| e["action"] == "app.created")
        .unwrap();
    assert_eq!(created["actor"], "dr-park", "no more DOCTOR const");
    let promoted = events
        .iter()
        .find(|e| e["action"] == "app.promoted")
        .unwrap();
    assert!(promoted["detail"]
        .as_str()
        .unwrap()
        .contains("co-signed Dr. J. Park (dr-park)"));
}

// ---------- P14 (closeout): the doctor's edit is the doctor's act ----------

#[tokio::test]
async fn iterate_audit_event_is_attributed_to_the_requesting_principal() {
    let router = strict_router(None);
    let id = create_app(&router, PARK, "post-op-monitor", "wound diary").await;
    let (status, _, _) = call(
        &router,
        "POST",
        &format!("/api/apps/{id}/iterate"),
        Some(PARK),
        Some(json!({"instruction": "flag rising pain to my inbox"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, audit, _) = call(
        &router,
        "GET",
        &format!("/api/apps/{id}/audit"),
        Some(PARK),
        None,
    )
    .await;
    let events = audit["events"].as_array().unwrap();
    let iterated = events
        .iter()
        .find(|e| e["action"] == "app.iterated")
        .expect("app.iterated recorded");
    // P14 resolved: the actor is the principal who asked for the edit; the
    // machine's contribution (which agent tier landed it) is the detail.
    assert_eq!(iterated["actor"], "dr-park", "{iterated}");
    let detail = iterated["detail"].as_str().unwrap();
    assert!(
        detail.contains("agent tier rules"),
        "the landing tier rides the detail: {detail}"
    );
    // The machine's own records keep the agent actor — attribution, not
    // erasure.
    let attempt = events
        .iter()
        .find(|e| e["action"] == "agent.attempt")
        .expect("agent.attempt recorded");
    assert_eq!(attempt["actor"], "agent");
}
