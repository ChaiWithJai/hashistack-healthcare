//! Control-plane API. Everything above it — doctor UI, a future CLI, a
//! hospital integration — is a client of these routes. No privileged UI.
//!
//! Identity (#10): every `/api` route resolves an authenticated
//! [`Principal`] (src/identity.rs) through the bearer-token middleware
//! below; `/`, `/health`, `/proof`, and the static UI stay open. All
//! app-scoped routes are tenant-scoped (cross-tenant ids answer 404 — never
//! disclosing existence — with an `auth.cross_tenant_denied` audit event),
//! role capabilities gate promotion/co-sign and the platform audit export
//! (403 + `auth.role_denied`), and the audit actor is the real principal id.
//!
//! TODO(#10): operator access — staff debugging a tenant's running app —
//! follows Boundary's Target/Session shape per decision 0005 (time-boxed
//! session against a policy object, recording flag, `termination_reason`;
//! never standing access). Design record only in this link: today the staff
//! role has no cross-tenant read of any kind.

use axum::extract::{Path, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use crate::agent::ScaffoldStep;
use crate::deploy;
use crate::eject;
use crate::gates;
use crate::identity::{Capability, Principal};
use crate::packs;
use crate::state::{
    now_unix, Addendum, AppRecord, AttemptRecord, DataSource, OpKind, OpStatus, Operation,
    Platform, SharedPlatform, Stage,
};
use crate::store::{self, StageTransition};

const DOCTOR_UI: &str = include_str!("../web/index.html");

pub fn router() -> Router {
    let platform: SharedPlatform = Arc::new(RwLock::new(Platform::new(packs::builtin_packs())));
    router_with_state(platform)
}

pub fn router_with_state(platform: SharedPlatform) -> Router {
    // Every /api route lives behind the identity middleware — coverage by
    // construction, so a new route cannot forget to authenticate. Health,
    // the doctor UI shell, and the original proof contract stay open.
    let api = Router::new()
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
        .route_layer(middleware::from_fn_with_state(
            platform.clone(),
            authenticate,
        ));
    Router::new()
        .route("/", get(doctor_ui))
        .route("/health", get(crate::health))
        .route("/proof/:workload", post(crate::proof))
        .merge(api)
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

fn unauthorized(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::UNAUTHORIZED, msg.into())
}

type ApiResult<T> = Result<Json<T>, ApiError>;

// ---------- identity (#10): authn middleware + tenancy/role guards ----------

/// Resolve the caller of an `/api` route to a [`Principal`] and stash it in
/// the request extensions. Behavior by mode (src/identity.rs module doc):
/// a bearer token maps to its declared principal (unknown token → 401, in
/// every mode); a MISSING header falls back to dr-osei only in dev (no
/// `IDENTITIES_FILE`), audited as `auth.dev_fallback` on first use per
/// boot; in strict mode it is 401. With `SESSION_IDLE_SECS` on, a session
/// idle past the limit is 401 + `auth.session_expired` — the platform
/// honoring its own auto-logoff gate.
async fn authenticate(
    State(platform): State<SharedPlatform>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let registry = platform.read().unwrap().identity.clone();
    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .map(|v| v.to_str().map(str::to_string))
        .transpose()
        .map_err(|_| unauthorized("unreadable Authorization header"))?;
    let principal = match header.as_deref() {
        Some(value) => {
            let token = value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| unauthorized("Authorization header must be `Bearer <token>`"))?;
            let principal = registry
                .by_token(token)
                .ok_or_else(|| unauthorized("unrecognized bearer token"))?
                .clone();
            expire_idle_session(&platform, &registry, &principal)?;
            principal
        }
        None => {
            let Some(principal) = registry.fallback().cloned() else {
                return Err(unauthorized(
                    "missing Authorization: Bearer <token> (IDENTITIES_FILE is set — no dev fallback)",
                ));
            };
            expire_idle_session(&platform, &registry, &principal)?;
            if registry.announce_dev_fallback() {
                let mut plat = platform.write().unwrap();
                plat.audit.record(
                    &principal.id,
                    "auth.dev_fallback",
                    "request carried no Authorization header and no IDENTITIES_FILE is \
                     configured — the embedded dev registry attributed it to dr-osei. \
                     Phase 0 dev convenience so the zero-config UI works; with \
                     IDENTITIES_FILE set (staging), the same request answers 401",
                    None,
                );
            }
            principal
        }
    };
    req.extensions_mut().insert(principal);
    Ok(next.run(req).await)
}

/// Session-idle enforcement for one token use: past `SESSION_IDLE_SECS`
/// the request is refused (401) and `auth.session_expired` lands in the
/// audit stream with the denied principal as actor. The denial itself is
/// the logoff boundary — the next request re-authenticates afresh.
fn expire_idle_session(
    platform: &SharedPlatform,
    registry: &crate::identity::Registry,
    principal: &Principal,
) -> Result<(), ApiError> {
    if let Err(idle) = registry.touch(&principal.token) {
        let mut plat = platform.write().unwrap();
        plat.audit.record(
            &principal.id,
            "auth.session_expired",
            format!(
                "session idle past {idle}s — automatic logoff (the platform honors its \
                 own auto-logoff gate); the next authenticated request starts a new session"
            ),
            None,
        );
        return Err(unauthorized(format!(
            "session expired after {idle}s idle — re-authenticate"
        )));
    }
    Ok(())
}

/// Resolve an app id under the caller's tenant. An id owned by another
/// tenant answers exactly like a nonexistent one — 404, existence never
/// disclosed across the boundary — and the denial is audited on the app's
/// (owning tenant's) stream with the denied principal as actor.
fn ensure_tenant(
    platform: &SharedPlatform,
    id: &str,
    principal: &Principal,
) -> Result<(), ApiError> {
    let mismatch = {
        let plat = platform.read().unwrap();
        match plat.apps.get(id) {
            None => return Err(not_found("app")),
            Some(app) => app.tenant != principal.tenant,
        }
    };
    if mismatch {
        let mut plat = platform.write().unwrap();
        plat.audit.record(
            &principal.id,
            "auth.cross_tenant_denied",
            format!(
                "principal of tenant {} addressed an app outside it — answered 404 \
                 (existence not disclosed)",
                principal.tenant
            ),
            Some(id),
        );
        return Err(not_found("app"));
    }
    Ok(())
}

/// The role capability gate (one check, not scattered ifs). Unlike the
/// tenancy guard this CAN be 403: in-tenant existence is already known to
/// the caller, so the denial discloses nothing — and it is audited as
/// `auth.role_denied` with the denied principal as actor.
fn require_capability(
    platform: &SharedPlatform,
    principal: &Principal,
    capability: Capability,
    app_id: Option<&str>,
) -> Result<(), ApiError> {
    if principal.role.allows(capability) {
        return Ok(());
    }
    let mut plat = platform.write().unwrap();
    plat.audit.record(
        &principal.id,
        "auth.role_denied",
        format!(
            "role {} may not {} — 403",
            principal.role.as_str(),
            capability.describe()
        ),
        app_id,
    );
    Err(ApiError(
        StatusCode::FORBIDDEN,
        format!(
            "role {} may not {}",
            principal.role.as_str(),
            capability.describe()
        ),
    ))
}

// ---------- the broker invariant (#8): no audit write, no operation ----------

/// Durably settle a load-bearing operation (classification in src/audit.rs):
/// write through to the control store (#7), then require ≥1 durable audit
/// sink to confirm every event through the current head (#8, Vault broker).
///
/// - Dev mode (no durable sink): returns Ok immediately after the (no-op)
///   write-through — byte-identical to the pre-#8 behavior.
/// - A stage transition the control DB refused fails here regardless of
///   other sinks (#7's rule: the state itself was not durable).
/// - Every failed sink lands an `audit.sink_failed` event in the in-memory
///   fallback — the record of the failure is never lost — and retries past
///   its own watermark on the next confirmation.
///
/// On Err the caller must revert its state change and answer 503: the
/// operation did not happen.
async fn settle_durable(
    platform: &SharedPlatform,
    app_ids: &[&str],
    transition: Option<StageTransition>,
) -> Result<(), String> {
    let is_stage_transition = transition.as_ref().is_some_and(|t| t.prior.is_some());
    if let Err(e) = store::write_through(platform, app_ids, transition).await {
        if is_stage_transition {
            return Err(format!(
                "control DB refused or missed the stage transition: {e:#}"
            ));
        }
        // Elsewhere a control-store miss degrades durability (retried on
        // the next write-through); the AUDIT record below still decides
        // whether the operation stands.
        tracing::warn!("control DB write-through failed (durability degraded): {e:#}");
    }

    let (broker, events, target) = {
        let plat = platform.read().unwrap();
        if !plat.broker.durable_configured() {
            return Ok(()); // dev mode: memory is the fallback AND the record
        }
        (
            plat.broker.clone(),
            plat.audit.events().to_vec(),
            plat.audit.head_seq(),
        )
    };
    let outcome = broker.confirm(&events, target).await;
    if !outcome.failed.is_empty() {
        let mut plat = platform.write().unwrap();
        for (name, err) in &outcome.failed {
            tracing::error!("audit sink {name} failed: {err}");
            plat.audit.record(
                "audit-broker",
                "audit.sink_failed",
                format!("sink {name} did not confirm through seq {target}: {err}"),
                app_ids.first().copied(),
            );
        }
    }
    if outcome.confirmed.is_empty() {
        let causes: Vec<String> = outcome
            .failed
            .iter()
            .map(|(n, e)| format!("{n}: {e}"))
            .collect();
        return Err(format!(
            "audit unavailable — no durable sink confirmed the write (seq {target}; {})",
            causes.join("; ")
        ));
    }
    Ok(())
}

/// The 503 a load-bearing operation answers when durability failed and its
/// state change was reverted.
fn audit_unavailable(what: &str, cause: String) -> ApiError {
    ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{what} — {cause}"))
}

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
    /// #10: tenancy is derived from the authenticated principal, never the
    /// request. The field survives for old clients as a validate-equal
    /// check — naming any other tenant is 422 (review-log P10).
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
    Extension(principal): Extension<Principal>,
    Json(req): Json<CreateApp>,
) -> ApiResult<CreatedApp> {
    // #10: the app's tenant IS the principal's tenant. A request naming a
    // different one is refused loudly rather than silently overridden.
    if let Some(requested) = req.tenant.as_deref() {
        if requested != principal.tenant {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "tenant is derived from the authenticated principal ({}) — \
                     omit the field or match it",
                    principal.tenant
                ),
            ));
        }
    }
    // Short write lock: resolve the pack, mint the id, promise the work.
    // The ladder climb itself runs with no lock held (F4).
    let (pack, id, name, tenant, ladder) = {
        let mut plat = platform.write().unwrap();
        let pack = plat
            .pack(&req.pack)
            .ok_or_else(|| not_found("pack"))?
            .clone();

        let name = req.name.clone().unwrap_or_else(|| pack.name.clone());
        let tenant = principal.tenant.clone();
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

        // The prompt is doctor-authored free text: it rides the sensitive
        // envelope (#8) — HMAC on every platform-wide surface, plaintext in
        // the doctor's own app-scoped view.
        plat.audit.record_sensitive(
            &principal.id,
            "app.created",
            format!("described from pack {}", pack.id),
            Some(&id),
            &[("prompt", req.prompt.clone())],
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

    // #7 write-through (the creation history row, prior NULL → sandbox) +
    // #8 broker invariant: the scaffold settle is load-bearing. If no
    // durable audit sink confirms `app.created`/`agent.scaffolded`, the
    // draft is withdrawn — an app that the audit stream cannot prove was
    // created does not exist.
    if let Err(cause) = settle_durable(
        &platform,
        &[&id],
        Some(StageTransition {
            app_id: id.clone(),
            prior: None,
            next: Stage::Sandbox,
            operation_id: None,
        }),
    )
    .await
    {
        platform.write().unwrap().apps.remove(&id);
        return Err(audit_unavailable("draft withdrawn", cause));
    }
    Ok(Json(CreatedApp { app, scaffold }))
}

/// #10: the list is tenant-scoped — a clinician sees their practice's apps
/// and nothing else. No 404 games needed here; filtering IS the boundary.
async fn list_apps(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<serde_json::Value> {
    let plat = platform.read().unwrap();
    let mut apps: Vec<&AppRecord> = plat
        .apps
        .values()
        .filter(|a| a.tenant == principal.tenant)
        .collect();
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(json!({ "apps": apps })))
}

async fn get_app(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<AppRecord> {
    ensure_tenant(&platform, &id, &principal)?;
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
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    Json(req): Json<Iterate>,
) -> ApiResult<serde_json::Value> {
    ensure_tenant(&platform, &id, &principal)?;
    let (pack, ladder, snapshot) = {
        let plat = platform.read().unwrap();
        let app = plat
            .apps
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found("app"))?;
        let pack = plat
            .pack(&app.pack)
            .cloned()
            .ok_or_else(|| not_found("app"))?;
        (pack, plat.ladder.clone(), app)
    };
    // The edit is an operation climbing the verified escalation ladder:
    // each tier's output is applied to a clone and gate-checked; only an
    // accepted edit is committed. The climb holds NO platform lock (F4) —
    // the apply re-acquires it and settles `concurrent-edit` if the record
    // moved. Top-of-ladder failure changes nothing.
    let outcome = ladder
        .run_iterate(&platform, &id, &req.instruction, &pack)
        .await;
    // Durably settle whatever the climb recorded (#7 + #8). An APPLIED edit
    // is load-bearing: it reverts if no durable sink confirms. A failed
    // climb keeps its own error below — its attempt records are retried
    // into the sinks on the next confirmation.
    let durable = settle_durable(&platform, &[&id], None).await;
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
    if let Err(cause) = durable {
        let mut plat = platform.write().unwrap();
        if let Some(a) = plat.apps.get_mut(&id) {
            *a = snapshot;
        }
        return Err(audit_unavailable("edit reverted", cause));
    }
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
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    Json(req): Json<Restore>,
) -> ApiResult<AppRecord> {
    ensure_tenant(&platform, &id, &principal)?;
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
            &principal.id,
            "app.restored",
            format!("restored checkpoint v{}", req.version),
            Some(&id),
        );
        app
    };
    // Best-effort by classification (src/audit.rs): restore is a
    // sandbox-only rebuild from scaffold + addenda, whose creation was
    // itself durably settled by the load-bearing operations above.
    store::write_through_or_warn(&platform, &[&id], None).await;
    Ok(Json(app))
}

// ---------- gate ----------

async fn gate_report(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    ensure_tenant(&platform, &id, &principal)?;
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
    Extension(principal): Extension<Principal>,
    Path((id, gate_id)): Path<(String, String)>,
) -> ApiResult<serde_json::Value> {
    ensure_tenant(&platform, &id, &principal)?;
    if !gates::known_gate(&gate_id) {
        return Err(not_found("gate"));
    }
    if !gates::gate_fixable(&gate_id) {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("gate {gate_id} cannot be auto-fixed — it needs a code change via iterate"),
        ));
    }
    let (wired_response, already, snapshot) = {
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
        let snapshot = app.clone();
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
        (app, !newly_wired, snapshot)
    };
    // Load-bearing (#8): a wired compliance control the audit stream cannot
    // prove was wired is unwired again.
    if let Err(cause) = settle_durable(&platform, &[&id], None).await {
        let mut plat = platform.write().unwrap();
        if let Some(a) = plat.apps.get_mut(&id) {
            *a = snapshot;
        }
        return Err(audit_unavailable("gate fix reverted", cause));
    }
    Ok(Json(
        json!({ "wired": gate_id, "already_wired": already, "app": wired_response }),
    ))
}

/// Automated platform review (storyboard 1c's co-sign card): evaluates every
/// gate except human-review itself, attaches the reviewer's note, and marks
/// the review control satisfied.
async fn review(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    ensure_tenant(&platform, &id, &principal)?;
    let (note, report, snapshot) = {
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
        let snapshot = app.clone();

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
        (note, report, snapshot)
    };
    // Load-bearing (#8): an attested review must be durably recorded — the
    // note and the satisfied human-review control revert otherwise.
    if let Err(cause) = settle_durable(&platform, &[&id], None).await {
        let mut plat = platform.write().unwrap();
        if let Some(a) = plat.apps.get_mut(&id) {
            *a = snapshot;
        }
        return Err(audit_unavailable("review reverted", cause));
    }
    Ok(Json(json!({ "reviewer_note": note, "report": report })))
}

// ---------- deploy ----------

#[derive(Deserialize)]
struct Promote {
    /// #10: the co-sign is the authenticated principal's act; this typed
    /// field survives only as a display-name check (must match the
    /// principal's registered name, or be omitted — deploy::promote).
    #[serde(default)]
    cosigner: Option<String>,
}

async fn promote(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    Json(req): Json<Promote>,
) -> ApiResult<serde_json::Value> {
    // #10: tenancy first (a cross-tenant id is 404, existence undisclosed),
    // then the role capability — releasing to real patients is a clinical
    // act, so staff answer 403 `auth.role_denied`.
    ensure_tenant(&platform, &id, &principal)?;
    require_capability(&platform, &principal, Capability::CoSignRelease, Some(&id))?;
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
        deploy::promote(app, &report, &principal, req.cosigner.as_deref(), alloc_id)
            .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
        // Staging (#2, #9): submit the rendered job to a real Nomad dev
        // agent, prove the tenant transit key, mount the tenant policy, and
        // issue + verify dynamic DB creds against a real Vault — no-op (and
        // no events) when NOMAD_ADDR / VAULT_ADDR+VAULT_TOKEN are unset.
        // NOTE: these are loopback dev-mode calls; a real client pool moves
        // them off the lock like the model tiers (F4 / #6).
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
        // #10: the promote event names the authenticated principal and the
        // digest the co-sign binds — the audit stream carries the whole
        // cryptographic act, not just a typed name.
        let attestation = app.attestation.as_ref().expect("promote set attestation");
        plat.audit.record(
            "deploy",
            "app.promoted",
            format!(
                "deploy v{} approved (preflight {}) — co-signed {} ({}) binding report digest {} — allocation {} in prod pool",
                app.current_version,
                report.summary(),
                attestation.cosigner,
                principal.id,
                attestation.report_digest.as_deref().unwrap_or("?"),
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

    // #7 + #8: a stage transition the durable store did not confirm DID NOT
    // HAPPEN (the DB trigger checks sandbox→live against app_valid_state
    // inside this write), and a promotion no durable audit sink recorded
    // did not happen either — the broker invariant. On any failure the
    // in-memory record reverts and the doctor sees 503.
    if let Err(cause) = settle_durable(
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
        plat.audit
            .record("deploy", "app.promotion_reverted", cause.clone(), Some(&id));
        return Err(audit_unavailable("promotion reverted", cause));
    }

    Ok(Json(json!({ "app": app, "report": report })))
}

async fn rollback(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<AppRecord> {
    // Tenant-scoped but role-open: withdrawing an app from use must never
    // wait on the clinician (safety beats ceremony) — staff may roll back.
    ensure_tenant(&platform, &id, &principal)?;
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
        // Staging (#2, #9): the real allocation must actually die. If Nomad
        // refuses the stop, the rollback is refused too — the record never
        // claims the sandbox while a real job still runs — and the database
        // lease is only revoked (and revocation proven) after the stop.
        let staging_events = match deploy::staging_rollback(&snapshot) {
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

    // #7 + #8: same rule as promote — the live→sandbox transition AND its
    // audit record must be durably confirmed or the rollback did not happen.
    if let Err(cause) = settle_durable(
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
        plat.audit
            .record("deploy", "app.rollback_reverted", cause.clone(), Some(&id));
        return Err(audit_unavailable("rollback reverted", cause));
    }
    Ok(Json(app))
}

// ---------- operate + audit ----------

async fn operate(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    ensure_tenant(&platform, &id, &principal)?;
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
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    ensure_tenant(&platform, &id, &principal)?;
    let plat = platform.read().unwrap();
    if !plat.apps.contains_key(&id) {
        return Err(not_found("app"));
    }
    Ok(Json(json!({ "operations": plat.operations_for_app(&id) })))
}

/// The doctor's own app-scoped stream — the TENANT side of the HMAC
/// boundary (#8, decision 0004): sensitive values render as the doctor's
/// own plaintext here, and only here. The platform-wide export below keeps
/// the `hmac-sha256:` form. #10 keys this surface on the requesting
/// principal's tenant: a cross-tenant caller gets 404, never the plaintext.
async fn app_audit(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    ensure_tenant(&platform, &id, &principal)?;
    let plat = platform.read().unwrap();
    if !plat.apps.contains_key(&id) {
        return Err(not_found("app"));
    }
    Ok(Json(
        json!({ "events": plat.audit.for_app_tenant_view(&id) }),
    ))
}

/// Ejection (#11): the whole app as an owned, documented, extendable bundle.
/// Docs are generated from the doctor's record; live apps embed the release
/// attestation, sandbox apps export too but marked draft — not released.
async fn export_app(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<eject::EjectionBundle> {
    // Tenant-scoped, role-open: the bundle is the tenant's own record
    // (plaintext side of the HMAC boundary), so 404 outside the tenant.
    ensure_tenant(&platform, &id, &principal)?;
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
            &principal.id,
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
    // Load-bearing (#8): the bundle is withheld unless the record that it
    // left the platform is durable. Nothing to revert — the append-only
    // `app.exported` event stands as the record of the refused attempt.
    if let Err(cause) = settle_durable(&platform, &[&id], None).await {
        return Err(audit_unavailable("export withheld", cause));
    }
    Ok(Json(bundle))
}

/// Platform-wide export — the CROSS-TENANT side of the HMAC boundary (#8):
/// doctor-authored free text appears only as `hmac-sha256:<hex>`, so the
/// stream stays searchable and correlatable without being disclosable.
/// #10: role-gated — clinicians may pull the security-review export, staff
/// answer 403 (`auth.role_denied`).
async fn export_audit(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
) -> Result<Response, ApiError> {
    require_capability(&platform, &principal, Capability::ExportPlatformAudit, None)?;
    let plat = platform.read().unwrap();
    Ok((
        [("content-type", "application/jsonl")],
        plat.audit.export_jsonl(),
    )
        .into_response())
}
