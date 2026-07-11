//! Control-plane API. Everything above it — doctor UI, a future CLI, a
//! hospital integration — is a client of these routes. No privileged UI.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use crate::agent::{AgentDriver, RuleBasedDriver, ScaffoldStep};
use crate::deploy;
use crate::gates;
use crate::packs;
use crate::state::{now_unix, Addendum, AppRecord, DataSource, Platform, SharedPlatform, Stage};

const DOCTOR_UI: &str = include_str!("../web/index.html");

/// The doctor's identity in Phase 0's single-practice demo tenancy.
const DOCTOR: &str = "dr-osei";
const DEFAULT_TENANT: &str = "meridian";

pub fn router() -> Router {
    let platform: SharedPlatform = Arc::new(RwLock::new(Platform::new(packs::builtin_packs())));
    router_with_state(platform)
}

pub fn router_with_state(platform: SharedPlatform) -> Router {
    Router::new()
        .route("/", get(doctor_ui))
        .route("/health", get(crate::health))
        .route("/proof/:workload", post(crate::proof))
        .route("/api/packs", get(list_packs))
        .route("/api/apps", get(list_apps).post(create_app))
        .route("/api/apps/:id", get(get_app))
        .route("/api/apps/:id/iterate", post(iterate))
        .route("/api/apps/:id/restore", post(restore))
        .route("/api/apps/:id/review", post(review))
        .route("/api/apps/:id/gate", get(gate_report))
        .route("/api/apps/:id/gate/:gate_id/fix", post(fix_gate))
        .route("/api/apps/:id/promote", post(promote))
        .route("/api/apps/:id/rollback", post(rollback))
        .route("/api/apps/:id/operate", get(operate))
        .route("/api/apps/:id/audit", get(app_audit))
        .route("/api/apps/:id/export", get(export_app))
        .route("/api/audit/export", get(export_audit))
        .with_state(platform)
}

async fn doctor_ui() -> Html<&'static str> {
    Html(DOCTOR_UI)
}

// ---------- errors ----------

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn not_found(what: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("{what} not found"))
}

type ApiResult<T> = Result<Json<T>, ApiError>;

// ---------- packs ----------

async fn list_packs(State(platform): State<SharedPlatform>) -> ApiResult<serde_json::Value> {
    let plat = platform.read().unwrap();
    Ok(Json(json!({ "packs": plat.packs })))
}

// ---------- describe → generate ----------

#[derive(Deserialize)]
struct CreateApp {
    prompt: String,
    pack: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tenant: Option<String>,
}

#[derive(Serialize)]
struct CreatedApp {
    app: AppRecord,
    scaffold: Vec<ScaffoldStep>,
}

async fn create_app(
    State(platform): State<SharedPlatform>,
    Json(req): Json<CreateApp>,
) -> ApiResult<CreatedApp> {
    let mut plat = platform.write().unwrap();
    let pack = plat
        .pack(&req.pack)
        .ok_or_else(|| not_found("pack"))?
        .clone();

    let name = req.name.unwrap_or_else(|| pack.name.clone());
    let tenant = req.tenant.unwrap_or_else(|| DEFAULT_TENANT.to_string());
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let id = if plat.apps.contains_key(&slug) {
        plat.mint_id(&slug)
    } else {
        slug
    };

    let scaffold = RuleBasedDriver.scaffold(&pack, &req.prompt);
    let controls: BTreeSet<String> = pack.prewired.iter().cloned().collect();

    let app = AppRecord {
        id: id.clone(),
        name,
        prompt: req.prompt,
        pack: pack.id.clone(),
        stage: Stage::Sandbox,
        data_source: DataSource::Synthetic(pack.synthetic_dataset.clone()),
        controls,
        external_calls: vec!["api.anthropic.com".to_string()],
        features: pack.scaffold.clone(),
        routes: pack.scaffold.len() as u32,
        addenda: vec![Addendum {
            version: 1,
            instruction: "initial draft from protocol".to_string(),
            reply: format!("scaffolded from pack {} — hipaa-core pre-wired", pack.id),
            added_feature: None,
            wired_controls: pack.prewired.clone(),
            at: now_unix(),
        }],
        current_version: 1,
        reviewer_note: None,
        allocation: None,
        attestation: None,
        tenant,
    };

    plat.audit.record(
        DOCTOR,
        "app.created",
        format!("described {:?} from pack {}", app.prompt, pack.id),
        Some(&id),
    );
    plat.audit.record(
        "agent",
        "agent.scaffolded",
        format!(
            "{} features from pack {}, sandbox pool, synthetic data only",
            app.features.len(),
            pack.id
        ),
        Some(&id),
    );

    plat.apps.insert(id.clone(), app.clone());
    Ok(Json(CreatedApp { app, scaffold }))
}

async fn list_apps(State(platform): State<SharedPlatform>) -> ApiResult<serde_json::Value> {
    let plat = platform.read().unwrap();
    let mut apps: Vec<&AppRecord> = plat.apps.values().collect();
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(json!({ "apps": apps })))
}

async fn get_app(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<AppRecord> {
    let plat = platform.read().unwrap();
    plat.apps
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("app"))
}

// ---------- iterate ----------

#[derive(Deserialize)]
struct Iterate {
    instruction: String,
}

async fn iterate(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
    Json(req): Json<Iterate>,
) -> ApiResult<serde_json::Value> {
    let mut plat = platform.write().unwrap();
    let required = plat
        .apps
        .get(&id)
        .and_then(|a| plat.pack(&a.pack))
        .map(|p| p.gates.clone())
        .ok_or_else(|| not_found("app"))?;
    let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;

    let reply = RuleBasedDriver.iterate(app, &req.instruction, &required);
    app.current_version += 1;
    let version = app.current_version;
    app.addenda.push(Addendum {
        version,
        instruction: req.instruction.clone(),
        reply: reply.message.clone(),
        added_feature: reply.added_feature.clone(),
        wired_controls: reply.wired_controls.clone(),
        at: now_unix(),
    });
    let app = app.clone();

    plat.audit.record(
        "agent",
        "app.iterated",
        format!("addendum {} — {:?}", version, req.instruction),
        Some(&id),
    );

    Ok(Json(json!({ "reply": reply, "app": app })))
}

#[derive(Deserialize)]
struct Restore {
    version: u32,
}

/// Restore a checkpoint by rebuilding the record from scaffold + addenda —
/// derived state, the immutability principle applied to app history.
async fn restore(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
    Json(req): Json<Restore>,
) -> ApiResult<AppRecord> {
    let mut plat = platform.write().unwrap();
    let scaffold = plat
        .apps
        .get(&id)
        .and_then(|a| plat.pack(&a.pack))
        .map(|p| p.scaffold.clone())
        .ok_or_else(|| not_found("app"))?;
    let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;
    if !app.version_exists(req.version) {
        return Err(not_found("version"));
    }
    if app.stage == Stage::Live {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "roll back the live allocation before restoring a sandbox checkpoint".to_string(),
        ));
    }

    app.addenda.retain(|a| a.version <= req.version);
    app.current_version = req.version;
    app.features = scaffold.clone();
    app.controls = BTreeSet::new();
    for addendum in &app.addenda {
        app.features.extend(addendum.added_feature.clone());
        app.controls.extend(addendum.wired_controls.iter().cloned());
    }
    app.routes = app.features.len() as u32;
    let app = app.clone();

    plat.audit.record(
        DOCTOR,
        "app.restored",
        format!("restored checkpoint v{}", req.version),
        Some(&id),
    );
    Ok(Json(app))
}

// ---------- gate ----------

async fn gate_report(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let plat = platform.read().unwrap();
    let app = plat.apps.get(&id).ok_or_else(|| not_found("app"))?;
    let pack = plat.pack(&app.pack).ok_or_else(|| not_found("pack"))?;
    let report = gates::preflight(app, &pack.gates);
    Ok(Json(json!({
        "report": report,
        "meter": gates::meter(&report),
        "reviewer_note": app.reviewer_note,
    })))
}

/// "fix it for me": the agent wires a fixable control and logs an addendum.
async fn fix_gate(
    State(platform): State<SharedPlatform>,
    Path((id, gate_id)): Path<(String, String)>,
) -> ApiResult<serde_json::Value> {
    if !gates::known_gate(&gate_id) {
        return Err(not_found("gate"));
    }
    if !gates::gate_fixable(&gate_id) {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("gate {gate_id} cannot be auto-fixed — it needs a code change via iterate"),
        ));
    }
    let mut plat = platform.write().unwrap();
    let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;
    let newly_wired = app.controls.insert(gate_id.clone());
    if newly_wired {
        app.current_version += 1;
        app.addenda.push(Addendum {
            version: app.current_version,
            instruction: format!("fix it for me: {gate_id}"),
            reply: format!("✓ wired {gate_id}"),
            added_feature: None,
            wired_controls: vec![gate_id.clone()],
            at: now_unix(),
        });
    }
    let app = app.clone();
    plat.audit.record(
        "agent",
        "gate.fixed",
        format!("wired control {gate_id}"),
        Some(&id),
    );
    Ok(Json(
        json!({ "wired": gate_id, "already_wired": !newly_wired, "app": app }),
    ))
}

/// Automated platform review (storyboard 1c's co-sign card): evaluates every
/// gate except human-review itself, attaches the reviewer's note, and marks
/// the review control satisfied.
async fn review(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let mut plat = platform.write().unwrap();
    let (required, tier) = plat
        .apps
        .get(&id)
        .and_then(|a| plat.pack(&a.pack))
        .map(|p| (p.gates.clone(), p.tier))
        .ok_or_else(|| not_found("app"))?;
    let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;

    let reviewable: Vec<String> = required
        .iter()
        .filter(|g| g.as_str() != "human-review")
        .cloned()
        .collect();
    let report = gates::preflight(app, &reviewable);
    let note = gates::reviewer_note(&report, tier);
    app.reviewer_note = Some(note.clone());
    if report.green {
        app.controls.insert("human-review".to_string());
    }
    plat.audit.record(
        "platform-reviewer",
        "review.completed",
        note.clone(),
        Some(&id),
    );
    Ok(Json(json!({ "reviewer_note": note, "report": report })))
}

// ---------- deploy ----------

#[derive(Deserialize)]
struct Promote {
    cosigner: String,
}

async fn promote(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
    Json(req): Json<Promote>,
) -> ApiResult<serde_json::Value> {
    let mut plat = platform.write().unwrap();
    let required = plat
        .apps
        .get(&id)
        .and_then(|a| plat.pack(&a.pack))
        .map(|p| p.gates.clone())
        .ok_or_else(|| not_found("app"))?;
    let alloc_id = plat.mint_id("a");
    let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;

    let report = gates::preflight(app, &required);
    deploy::promote(app, &report, &req.cosigner, alloc_id)
        .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
    let app = app.clone();

    plat.audit.record(
        "gate-engine",
        "gate.passed",
        format!(
            "preflight {} green at v{}",
            report.summary(),
            report.app_version
        ),
        Some(&id),
    );
    plat.audit.record(
        "deploy",
        "app.promoted",
        format!(
            "deploy v{} approved (preflight {}) — co-signed {} — allocation {} in prod pool",
            app.current_version,
            report.summary(),
            req.cosigner.trim(),
            app.allocation
                .as_ref()
                .map(|a| a.id.as_str())
                .unwrap_or("?"),
        ),
        Some(&id),
    );

    Ok(Json(json!({ "app": app, "report": report })))
}

async fn rollback(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<AppRecord> {
    let mut plat = platform.write().unwrap();
    let synthetic = plat
        .apps
        .get(&id)
        .and_then(|a| plat.pack(&a.pack))
        .map(|p| p.synthetic_dataset.clone())
        .ok_or_else(|| not_found("app"))?;
    let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;
    deploy::rollback(app, &synthetic).map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
    let app = app.clone();
    plat.audit.record(
        "deploy",
        "app.rolled_back",
        "allocation destroyed; app returned to sandbox on synthetic data",
        Some(&id),
    );
    Ok(Json(app))
}

// ---------- operate + audit ----------

async fn operate(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let plat = platform.read().unwrap();
    let app = plat.apps.get(&id).ok_or_else(|| not_found("app"))?;
    let live = app.stage == Stage::Live;
    Ok(Json(json!({
        "stage": app.stage,
        "allocation": app.allocation,
        "metrics": {
            "uptime_pct": if live { 100.0 } else { 0.0 },
            "p95_ms": if live { 120 } else { 0 },
            "healthy": app.allocation.as_ref().map(|a| a.healthy).unwrap_or(false),
        },
    })))
}

async fn app_audit(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let plat = platform.read().unwrap();
    if !plat.apps.contains_key(&id) {
        return Err(not_found("app"));
    }
    Ok(Json(json!({ "events": plat.audit.for_app(&id) })))
}

async fn export_app(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let plat = platform.read().unwrap();
    let app = plat.apps.get(&id).ok_or_else(|| not_found("app"))?;
    let job = deploy::render_job(app).map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(json!({
        "nomad_job": job,
        "portability": "exports as the monorepo shape: Dockerfile + render.yaml / fly.toml / kamal deploy.yml — no hostage code",
    })))
}

async fn export_audit(State(platform): State<SharedPlatform>) -> impl IntoResponse {
    let plat = platform.read().unwrap();
    (
        [("content-type", "application/jsonl")],
        plat.audit.export_jsonl(),
    )
}
