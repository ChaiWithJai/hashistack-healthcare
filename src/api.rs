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

use crate::agent::ScaffoldStep;
use crate::deploy;
use crate::eject;
use crate::gates;
use crate::packs;
use crate::state::{
    now_unix, Addendum, AppRecord, AttemptRecord, DataSource, OpKind, OpStatus, Operation,
    Platform, SharedPlatform, Stage,
};
use crate::store::{self, StageTransition};

const DOCTOR_UI: &str = include_str!("../web/index.html");

/// The doctor's identity in Phase 0's single-practice demo tenancy.
/// TODO(#10): every route is unauthenticated and actions attribute to this
/// constant. Real identity: OIDC per request, tenant scoping on every record,
/// co-sign as a cryptographic act binding identity + report digest.
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
        .route("/api/apps/:id/operations", get(app_operations))
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
    // Short write lock: resolve the pack, mint the id, promise the work.
    // The ladder climb itself runs with no lock held (F4).
    let (pack, id, name, tenant, ladder) = {
        let mut plat = platform.write().unwrap();
        let pack = plat
            .pack(&req.pack)
            .ok_or_else(|| not_found("pack"))?
            .clone();

        let name = req.name.clone().unwrap_or_else(|| pack.name.clone());
        let tenant = req
            .tenant
            .clone()
            .unwrap_or_else(|| DEFAULT_TENANT.to_string());
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

        plat.audit.record(
            DOCTOR,
            "app.created",
            format!("described {:?} from pack {}", req.prompt, pack.id),
            Some(&id),
        );
        let ladder = plat.ladder.clone();
        (pack, id, name, tenant, ladder)
    };

    // The scaffold is an operation upserted Running before the first driver
    // runs (Waypoint upsert-first), then verified up the escalation ladder.
    let scaffold = ladder
        .run_scaffold(&platform, &id, &pack, &req.prompt)
        .await
        .map_err(|f| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!(
                    "scaffold failed at every tier (op {}): {}",
                    f.op_id, f.reason
                ),
            )
        })?;
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

    {
        let mut plat = platform.write().unwrap();
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
    }

    // #7 write-through: the new record plus its creation history row
    // (prior NULL → sandbox). Not a stage transition — a failure degrades
    // durability and is logged, never blocks the doctor's draft.
    store::write_through_or_warn(
        &platform,
        &[&id],
        Some(StageTransition {
            app_id: id.clone(),
            prior: None,
            next: Stage::Sandbox,
            operation_id: None,
        }),
    )
    .await;
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
    let (pack, ladder) = {
        let plat = platform.read().unwrap();
        let pack = plat
            .apps
            .get(&id)
            .and_then(|a| plat.pack(&a.pack))
            .cloned()
            .ok_or_else(|| not_found("app"))?;
        (pack, plat.ladder.clone())
    };
    // The edit is an operation climbing the verified escalation ladder:
    // each tier's output is applied to a clone and gate-checked; only an
    // accepted edit is committed. The climb holds NO platform lock (F4) —
    // the apply re-acquires it and settles `concurrent-edit` if the record
    // moved. Top-of-ladder failure changes nothing.
    let outcome = ladder
        .run_iterate(&platform, &id, &req.instruction, &pack)
        .await;
    // Persist whatever the climb recorded — settled attempts and audit on
    // failure, the applied edit on success (#7; no stage change here).
    store::write_through_or_warn(&platform, &[&id], None).await;
    let (reply, app, _op_id) = outcome.map_err(|f| {
        if f.is_concurrent_edit() {
            ApiError(
                StatusCode::CONFLICT,
                format!(
                    "the app changed while the edit was verified (op {}) — retry the instruction",
                    f.op_id
                ),
            )
        } else {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!(
                    "iterate failed at every tier (op {}): {}",
                    f.op_id, f.reason
                ),
            )
        }
    })?;
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
    let app = {
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
        // Restore never changes stage, so it consults the transition table
        // by refusing to run outside the sandbox: editing a live record
        // without the live→sandbox transition (rollback) is exactly what
        // VALID_STAGE_TRANSITIONS makes impossible.
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
        app
    };
    store::write_through_or_warn(&platform, &[&id], None).await;
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
    let (wired_response, already) = {
        let mut plat = platform.write().unwrap();
        if !plat.apps.contains_key(&id) {
            return Err(not_found("app"));
        }

        // Even a deterministic fix is an operation: upserted Running before
        // the mutation so an interrupted fix is visible, settled with one
        // rules-tier attempt after it. The wiring is platform code, so no
        // ladder climb.
        let started = now_unix();
        let mut op = Operation {
            op_id: plat.mint_id("op"),
            app_id: id.clone(),
            kind: OpKind::Fix,
            status: OpStatus::Running,
            attempts: Vec::new(),
            started_at: started,
            finished_at: None,
        };
        plat.upsert_operation(op.clone());

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

        op.attempts.push(AttemptRecord {
            tier: "rules".to_string(),
            started_at: started,
            finished_at: now_unix(),
            verdict: "accepted".to_string(),
            reason: (!newly_wired).then(|| "already wired".to_string()),
        });
        op.status = OpStatus::Success;
        op.finished_at = Some(now_unix());
        let op_id = op.op_id.clone();
        plat.upsert_operation(op);

        plat.audit.record(
            "agent",
            "agent.attempt",
            format!(
                "op {op_id} fix v{} tier=rules verdict=accepted → applied",
                app.current_version
            ),
            Some(&id),
        );
        plat.audit.record(
            "agent",
            "gate.fixed",
            format!("wired control {gate_id}"),
            Some(&id),
        );
        (app, !newly_wired)
    };
    store::write_through_or_warn(&platform, &[&id], None).await;
    Ok(Json(
        json!({ "wired": gate_id, "already_wired": already, "app": wired_response }),
    ))
}

/// Automated platform review (storyboard 1c's co-sign card): evaluates every
/// gate except human-review itself, attaches the reviewer's note, and marks
/// the review control satisfied.
async fn review(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let (note, report) = {
        let mut plat = platform.write().unwrap();
        let pack = plat
            .apps
            .get(&id)
            .and_then(|a| plat.pack(&a.pack))
            .cloned()
            .ok_or_else(|| not_found("app"))?;
        let (required, tier) = (pack.gates.clone(), pack.tier);
        // Review routing is policy-recorded like every other decision; the
        // deterministic reviewer note stands in for the frontier reviewer.
        plat.audit.record(
            "agent",
            "agent.routed",
            format!(
                "per {}: review→{} (phase 0: deterministic reviewer note)",
                pack.routing_source(),
                pack.routing_policy().review
            ),
            Some(&id),
        );
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
        (note, report)
    };
    store::write_through_or_warn(&platform, &[&id], None).await;
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
    let (app, report, snapshot) = {
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
        // Snapshot so a failed staging submission reverts the whole
        // promotion: the record must never claim "live" when real
        // infrastructure said no.
        let snapshot = app.clone();
        deploy::promote(app, &report, &req.cosigner, alloc_id)
            .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
        // Staging (#2): submit the rendered job to a real Nomad dev agent
        // and prove the tenant transit key against a real Vault — no-op
        // (and no events) when NOMAD_ADDR / VAULT_ADDR+VAULT_TOKEN are
        // unset. NOTE: these are loopback dev-mode calls; a real client
        // pool moves them off the lock like the model tiers (F4 / #6).
        let staging_events = match deploy::staging_promote(app) {
            Ok(events) => events,
            Err(e) => {
                *app = snapshot;
                return Err(ApiError(
                    StatusCode::BAD_GATEWAY,
                    format!("staging submission failed — promotion reverted: {e}"),
                ));
            }
        };
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
        for (action, detail) in staging_events {
            plat.audit.record("deploy", &action, detail, Some(&id));
        }
        (app, report, snapshot)
    };

    // #7, the broker-invariant precursor: a stage transition that the
    // durable store did not confirm DID NOT HAPPEN. The DB trigger checks
    // sandbox→live against app_valid_state inside this write; on any
    // failure the in-memory record reverts and the doctor sees 503.
    if let Err(e) = store::write_through(
        &platform,
        &[&id],
        Some(StageTransition {
            app_id: id.clone(),
            prior: Some(Stage::Sandbox),
            next: Stage::Live,
            operation_id: None,
        }),
    )
    .await
    {
        let mut plat = platform.write().unwrap();
        if let Some(a) = plat.apps.get_mut(&id) {
            *a = snapshot;
        }
        plat.audit.record(
            "deploy",
            "app.promotion_reverted",
            format!("control DB refused or missed the stage transition: {e:#}"),
            Some(&id),
        );
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("control DB write failed — promotion reverted: {e:#}"),
        ));
    }

    Ok(Json(json!({ "app": app, "report": report })))
}

async fn rollback(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<AppRecord> {
    let (app, snapshot) = {
        let mut plat = platform.write().unwrap();
        let synthetic = plat
            .apps
            .get(&id)
            .and_then(|a| plat.pack(&a.pack))
            .map(|p| p.synthetic_dataset.clone())
            .ok_or_else(|| not_found("app"))?;
        let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;
        let snapshot = app.clone();
        deploy::rollback(app, &synthetic)
            .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
        // Staging (#2): the real allocation must actually die. If Nomad
        // refuses the stop, the rollback is refused too — the record never
        // claims the sandbox while a real job still runs.
        let staging_events = match deploy::staging_rollback(&id, &snapshot.tenant) {
            Ok(events) => events,
            Err(e) => {
                *app = snapshot;
                return Err(ApiError(
                    StatusCode::BAD_GATEWAY,
                    format!("staging rollback failed — allocation kept: {e}"),
                ));
            }
        };
        let app = app.clone();
        plat.audit.record(
            "deploy",
            "app.rolled_back",
            "allocation destroyed; app returned to sandbox on synthetic data",
            Some(&id),
        );
        for (action, detail) in staging_events {
            plat.audit.record("deploy", &action, detail, Some(&id));
        }
        (app, snapshot)
    };

    // #7: same broker-invariant precursor as promote — the live→sandbox
    // transition must be durably confirmed or it did not happen.
    if let Err(e) = store::write_through(
        &platform,
        &[&id],
        Some(StageTransition {
            app_id: id.clone(),
            prior: Some(Stage::Live),
            next: Stage::Sandbox,
            operation_id: None,
        }),
    )
    .await
    {
        let mut plat = platform.write().unwrap();
        if let Some(a) = plat.apps.get_mut(&id) {
            *a = snapshot;
        }
        plat.audit.record(
            "deploy",
            "app.rollback_reverted",
            format!("control DB refused or missed the stage transition: {e:#}"),
            Some(&id),
        );
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("control DB write failed — rollback reverted: {e:#}"),
        ));
    }
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

/// Operation rows for one app — the routing record. A `running` or
/// `escalated` row with no terminal successor is the crash-visible trace of
/// an interrupted agent action (Waypoint upsert-first, steering §4).
async fn app_operations(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let plat = platform.read().unwrap();
    if !plat.apps.contains_key(&id) {
        return Err(not_found("app"));
    }
    Ok(Json(json!({ "operations": plat.operations_for_app(&id) })))
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

/// Ejection (#11): the whole app as an owned, documented, extendable bundle.
/// Docs are generated from the doctor's record; live apps embed the release
/// attestation, sandbox apps export too but marked draft — not released.
async fn export_app(
    State(platform): State<SharedPlatform>,
    Path(id): Path<String>,
) -> ApiResult<eject::EjectionBundle> {
    let bundle = {
        let mut plat = platform.write().unwrap();
        let app = plat
            .apps
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found("app"))?;
        let pack = plat
            .pack(&app.pack)
            .cloned()
            .ok_or_else(|| not_found("pack"))?;
        let bundle = eject::bundle(&app, &pack, &plat.audit.for_app(&id));
        plat.audit.record(
            DOCTOR,
            "app.exported",
            format!(
                "ejection bundle: {} files, docs from the record, pack {}-template derived — no hostage code",
                bundle.files.len(),
                app.id
            ),
            Some(&id),
        );
        bundle
    };
    store::write_through_or_warn(&platform, &[], None).await;
    Ok(Json(bundle))
}

async fn export_audit(State(platform): State<SharedPlatform>) -> impl IntoResponse {
    let plat = platform.read().unwrap();
    (
        [("content-type", "application/jsonl")],
        plat.audit.export_jsonl(),
    )
}
