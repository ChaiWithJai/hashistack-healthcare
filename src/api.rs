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
        .route("/auth/config", get(auth_config))
        .route("/health", get(crate::health))
        .route("/proof/:workload", post(crate::proof))
        .merge(api)
        .with_state(platform)
}

async fn doctor_ui() -> Html<&'static str> {
    Html(DOCTOR_UI)
}

#[derive(Serialize)]
struct AuthConfig {
    mode: &'static str,
    publishable_key: Option<String>,
}

async fn auth_config() -> Json<AuthConfig> {
    let publishable_key = std::env::var("CLERK_PUBLISHABLE_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    Json(AuthConfig {
        mode: if publishable_key.is_some() {
            "clerk"
        } else {
            "static"
        },
        publishable_key,
    })
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
    let (registry, clerk) = {
        let platform = platform.read().unwrap();
        (platform.identity.clone(), platform.clerk.clone())
    };
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
            let (principal, session_key) = if let Some(clerk) = clerk.as_ref() {
                let session = clerk
                    .verify(token)
                    .await
                    .map_err(|_| unauthorized("invalid or expired session"))?;
                let principal = registry
                    .by_id(&session.principal_id)
                    .ok_or_else(|| unauthorized("user is not provisioned"))?
                    .clone();
                (principal, session.session_id)
            } else {
                let principal = registry
                    .by_token(token)
                    .ok_or_else(|| unauthorized("unrecognized bearer token"))?
                    .clone();
                (principal, token.to_string())
            };
            expire_idle_session(&platform, &registry, &principal, &session_key)?;
            principal
        }
        None => {
            if clerk.is_some() {
                return Err(unauthorized("authentication required"));
            }
            let Some(principal) = registry.fallback().cloned() else {
                return Err(unauthorized(
                    "missing Authorization: Bearer <token> (IDENTITIES_FILE is set — no dev fallback)",
                ));
            };
            let session_key = principal.token.clone();
            expire_idle_session(&platform, &registry, &principal, &session_key)?;
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
    session_key: &str,
) -> Result<(), ApiError> {
    if let Err(idle) = registry.touch(session_key) {
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

async fn serialize_app_mutation(
    platform: &SharedPlatform,
    app_id: &str,
) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = platform.write().unwrap().app_lock(app_id);
    lock.lock_owned().await
}

fn refuse_during_cleanup(platform: &SharedPlatform, app_id: &str) -> Result<(), ApiError> {
    let cleanup_pending = platform
        .read()
        .unwrap()
        .apps
        .get(app_id)
        .and_then(|app| app.allocation.as_ref())
        .is_some_and(|allocation| allocation.cleanup_pending);
    if cleanup_pending {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "rollback cleanup is pending; only rollback retry, operate, and audit are available"
                .to_string(),
        ));
    }
    Ok(())
}

/// The 503 a load-bearing operation answers when durability failed and its
/// state change was reverted.
fn audit_unavailable(what: &str, cause: String) -> ApiError {
    ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{what} — {cause}"))
}

/// Compensate a failed durable write only when the record is still exactly
/// the value this request installed. A blind snapshot restore can erase a
/// newer concurrent edit/review/promotion while an audit sink is timing out.
fn restore_if_unchanged(
    platform: &SharedPlatform,
    app_id: &str,
    installed: &AppRecord,
    prior: AppRecord,
) -> bool {
    let mut plat = platform.write().unwrap();
    let Some(current) = plat.apps.get_mut(app_id) else {
        return false;
    };
    let unchanged = serde_json::to_vec(&*current).ok() == serde_json::to_vec(installed).ok();
    if unchanged {
        *current = prior;
    }
    unchanged
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

        // Refusal surface (#12, GOAL.md bar 7): the four RFC 0001
        // out-of-scope shapes are refused at describe time with a WRITTEN
        // reason — nothing is scaffolded, no app id is minted, and the
        // refusal itself is on the record (`app.refused`, prompt riding the
        // sensitive envelope like `app.created`'s).
        if let Some(refusal) = crate::refusals::screen(&req.prompt, &pack) {
            plat.audit.record_sensitive(
                &principal.id,
                "app.refused",
                format!(
                    "describe refused — RFC 0001 use case {} ({}): {}",
                    refusal.rfc_use_case, refusal.class, refusal.reason
                ),
                None,
                &[("prompt", req.prompt.clone())],
            );
            return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, refusal.reason));
        }

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
        let id = plat.reserve_app_id(&slug);

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
            platform.write().unwrap().release_app_id(&id);
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
        plat.release_app_id(&id);
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
    let _serial = serialize_app_mutation(&platform, &id).await;
    refuse_during_cleanup(&platform, &id)?;
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
        .run_iterate(&platform, &id, &req.instruction, &pack, &principal.id)
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
        restore_if_unchanged(&platform, &id, &app, snapshot);
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
    let _serial = serialize_app_mutation(&platform, &id).await;
    refuse_during_cleanup(&platform, &id)?;
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
    let _serial = serialize_app_mutation(&platform, &id).await;
    refuse_during_cleanup(&platform, &id)?;
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
        restore_if_unchanged(&platform, &id, &wired_response, snapshot);
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
    let _serial = serialize_app_mutation(&platform, &id).await;
    refuse_during_cleanup(&platform, &id)?;
    let (note, report, reviewed_app, snapshot) = {
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
        let reviewed_app = app.clone();
        plat.audit.record(
            "platform-reviewer",
            "review.completed",
            note.clone(),
            Some(&id),
        );
        (note, report, reviewed_app, snapshot)
    };
    // Load-bearing (#8): an attested review must be durably recorded — the
    // note and the satisfied human-review control revert otherwise.
    if let Err(cause) = settle_durable(&platform, &[&id], None).await {
        restore_if_unchanged(&platform, &id, &reviewed_app, snapshot);
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
    /// Explicit escape hatch for demos whose report contains labeled stubs.
    /// This publishes only to the synthetic-demo pool and never changes the
    /// app's data source to tenant data.
    #[serde(default)]
    synthetic_demo: bool,
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
    let _serial = serialize_app_mutation(&platform, &id).await;
    refuse_during_cleanup(&platform, &id)?;
    require_capability(&platform, &principal, Capability::CoSignRelease, Some(&id))?;

    // Refused release attempts are security events, not transient HTTP
    // errors. Record the principal, exact report, and blockers before
    // returning, and require the configured durable audit broker to confirm
    // the event just like a successful promotion.
    let denied = {
        let mut plat = platform.write().unwrap();
        let required = plat
            .apps
            .get(&id)
            .and_then(|a| plat.pack(&a.pack))
            .map(|p| p.gates.clone())
            .ok_or_else(|| not_found("app"))?;
        let app = plat.apps.get(&id).ok_or_else(|| not_found("app"))?;
        let report = gates::preflight(app, &required);
        let blockers = report.promotion_blockers(req.synthetic_demo);
        if blockers.is_empty() {
            None
        } else {
            let detail = format!(
                "principal {} denied promotion of app {} at v{}; report {} ({}) digest {}; blockers: {}",
                principal.id,
                id,
                report.app_version,
                report.summary(),
                if req.synthetic_demo { "synthetic-demo requested" } else { "real-data release requested" },
                gates::report_digest(&report),
                blockers.join("; ")
            );
            plat.audit.record(
                &principal.id,
                "gate.promotion_denied",
                detail.clone(),
                Some(&id),
            );
            Some(detail)
        }
    };
    if let Some(detail) = denied {
        if let Err(cause) = settle_durable(&platform, &[&id], None).await {
            return Err(audit_unavailable(
                "promotion denial could not be durably recorded",
                cause,
            ));
        }
        return Err(ApiError(
            StatusCode::CONFLICT,
            format!("deploy locked: {detail}"),
        ));
    }

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
        deploy::promote(
            app,
            &report,
            &principal,
            req.cosigner.as_deref(),
            alloc_id,
            req.synthetic_demo,
        )
        .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
        // Staging (#2, #9): submit the rendered job to a real Nomad dev
        // agent, prove the tenant transit key, mount the tenant policy, and
        // issue + verify dynamic DB creds against a real Vault — no-op (and
        // no events) when NOMAD_ADDR / VAULT_ADDR+VAULT_TOKEN are unset.
        // NOTE: these are loopback dev-mode calls; a real client pool moves
        // them off the lock like the model tiers (F4 / #6).
        let staging_events = match if req.synthetic_demo {
            Ok(Vec::new())
        } else {
            deploy::staging_promote(app)
        } {
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
                "deploy v{} approved (preflight {}) — co-signed {} ({}) binding report digest {} — allocation {} in {} pool",
                app.current_version,
                report.summary(),
                attestation.cosigner,
                principal.id,
                attestation.report_digest.as_deref().unwrap_or("?"),
                app.allocation
                    .as_ref()
                    .map(|a| a.id.as_str())
                    .unwrap_or("?"),
                app.allocation
                    .as_ref()
                    .map(|a| a.pool.as_str())
                    .unwrap_or("unknown"),
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
        restore_if_unchanged(&platform, &id, &app, snapshot);
        let mut plat = platform.write().unwrap();
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
    let serial = serialize_app_mutation(&platform, &id).await;

    // The request is only an observer of the rollback worker. Moving the
    // per-app guard into an owned task means a disconnected client cannot
    // cancel the saga between an external side effect and its durable
    // checkpoint; the worker continues to a truthful terminal state while
    // later mutations of this app remain serialized behind it.
    tokio::spawn(rollback_serialized(platform, id, serial))
        .await
        .map_err(|error| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rollback worker panicked: {error}"),
            )
        })?
}

async fn rollback_serialized(
    platform: SharedPlatform,
    id: String,
    _serial: tokio::sync::OwnedMutexGuard<()>,
) -> ApiResult<AppRecord> {
    let real_cleanup = {
        let state = platform.read().unwrap();
        let app = state.apps.get(&id).ok_or_else(|| not_found("app"))?;
        let real_cleanup = app
            .allocation
            .as_ref()
            .is_some_and(|allocation| allocation.has_external_cleanup_handles());
        if real_cleanup && state.store.is_none() {
            return Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "real rollback requires CONTROL_DB_URL so cleanup intent survives restart"
                    .to_string(),
            ));
        }
        real_cleanup
    };

    // Persist the cleanup intent before the first irreversible external
    // action. A crash can therefore resume from the control DB instead of
    // reloading a stale apparently-running allocation.
    let (snapshot, intent, newly_requested, synthetic) = {
        let mut plat = platform.write().unwrap();
        let synthetic = plat
            .apps
            .get(&id)
            .and_then(|a| plat.pack(&a.pack))
            .map(|p| p.synthetic_dataset.clone())
            .ok_or_else(|| not_found("app"))?;
        let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;
        let snapshot = app.clone();
        let mut validation = snapshot.clone();
        deploy::rollback(&mut validation, &synthetic)
            .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
        let newly_requested = !app
            .allocation
            .as_ref()
            .is_some_and(|allocation| allocation.cleanup_pending);
        if newly_requested {
            if let Some(allocation) = app.allocation.as_mut() {
                allocation.healthy = false;
                allocation.cleanup_pending = true;
                allocation.cleanup_workload_stopped = false;
                allocation.cleanup_error = None;
            }
        }
        let intent = app.clone();
        if newly_requested {
            plat.audit.record(
                "deploy",
                "app.rollback_requested",
                "durable cleanup intent recorded before workload stop",
                Some(&id),
            );
        }
        (snapshot, intent, newly_requested, synthetic)
    };

    if newly_requested {
        if real_cleanup {
            if let Err(cause) = store::write_through(&platform, &[&id], None).await {
                restore_if_unchanged(&platform, &id, &intent, snapshot);
                return Err(audit_unavailable(
                    "rollback intent was not persisted to the control DB",
                    format!("{cause:#}"),
                ));
            }
        }
        if let Err(cause) = settle_durable(&platform, &[&id], None).await {
            // A real cleanup intent already reached the control DB. Keep the
            // same truthful in-memory intent when only audit confirmation
            // failed; synthetic rollback has no irreversible work and may
            // safely revert its candidate.
            if !real_cleanup {
                restore_if_unchanged(&platform, &id, &intent, snapshot);
            }
            return Err(audit_unavailable("rollback intent not confirmed", cause));
        }
    }

    let current = platform
        .read()
        .unwrap()
        .apps
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found("app"))?;
    let cleanup_driver = platform.read().unwrap().cleanup_driver.clone();
    let staging_events =
        match tokio::task::spawn_blocking(move || cleanup_driver.rollback(&current))
            .await
            .map_err(|error| {
                ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cleanup worker panicked: {error}"),
                )
            })? {
            Ok(events) => events,
            Err(failure) => {
                let stopped = failure.workload_stopped;
                {
                    let mut plat = platform.write().unwrap();
                    let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;
                    if let Some(allocation) = app.allocation.as_mut() {
                        allocation.healthy = false;
                        allocation.cleanup_pending = true;
                        allocation.cleanup_workload_stopped = stopped;
                        // Persist a bounded public code, never backend response text.
                        allocation.cleanup_error = Some(
                            if stopped {
                                "credential-cleanup-failed"
                            } else {
                                "workload-stop-failed"
                            }
                            .to_string(),
                        );
                    }
                    plat.audit.record(
                        "deploy",
                        "app.rollback_cleanup_pending",
                        if stopped {
                            "workload stopped; credential cleanup must be retried"
                        } else {
                            "workload stop was not confirmed; rollback must be retried"
                        },
                        Some(&id),
                    );
                }
                if let Err(cause) = settle_durable(&platform, &[&id], None).await {
                    return Err(audit_unavailable(
                        "rollback cleanup progress could not be durably recorded",
                        cause,
                    ));
                }
                tracing::error!(app_id = %id, error = %failure, "rollback cleanup failed");
                return Err(ApiError(
                    StatusCode::BAD_GATEWAY,
                    if stopped {
                        "workload stopped; credential cleanup pending and rollback may be retried"
                    } else {
                        "workload stop was not confirmed; rollback may be retried"
                    }
                    .to_string(),
                ));
            }
        };

    // Checkpoint externally verified cleanup before publishing Sandbox. If
    // the later stage transition fails, retry retains the confirmed stop.
    let verified_cleanup = {
        let mut plat = platform.write().unwrap();
        let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;
        if let Some(allocation) = app.allocation.as_mut() {
            allocation.healthy = false;
            allocation.cleanup_pending = true;
            allocation.cleanup_workload_stopped = true;
            allocation.cleanup_error = None;
        }
        let verified = app.clone();
        plat.audit.record(
            "deploy",
            "app.rollback_cleanup_verified",
            "workload stop and credential cleanup verified; sandbox transition pending",
            Some(&id),
        );
        verified
    };
    if real_cleanup {
        if let Err(cause) = store::write_through(&platform, &[&id], None).await {
            return Err(audit_unavailable(
                "verified rollback cleanup was not persisted to the control DB",
                format!("{cause:#}"),
            ));
        }
    }
    if let Err(cause) = settle_durable(&platform, &[&id], None).await {
        return Err(audit_unavailable(
            "verified rollback cleanup was not durably confirmed",
            cause,
        ));
    }

    let (app, cleanup_snapshot) = {
        let mut plat = platform.write().unwrap();
        let app = plat.apps.get_mut(&id).ok_or_else(|| not_found("app"))?;
        let cleanup_snapshot = verified_cleanup;
        deploy::rollback(app, &synthetic)
            .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
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
        (app, cleanup_snapshot)
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
        restore_if_unchanged(&platform, &id, &app, cleanup_snapshot);
        let mut plat = platform.write().unwrap();
        plat.audit
            .record("deploy", "app.rollback_reverted", cause.clone(), Some(&id));
        return Err(audit_unavailable("rollback reverted", cause));
    }
    Ok(Json(app))
}

// ---------- operate + audit ----------

/// Operate (#6, the honest slice of real allocations): the response carries
/// Nomad's dual status axes. `desired_state` is the platform record's claim
/// (live → running, sandbox → stopped); `observed_state` is polled from the
/// real Nomad job when `NOMAD_ADDR` is set (`status_source: "nomad"` — on
/// the one-machine dev agent an honest `pending` is expected, since
/// `role=prod` is unsatisfiable there) and mirrors desired in simulated
/// mode (`status_source: "simulated"` — labeled, never claimed). The poll
/// runs on the blocking pool with NO platform lock held (F4), and a
/// configured-but-unreachable Nomad answers 502 rather than a false
/// "running": the platform never claims an observation it didn't make.
/// TODO(#6): release≠deploy and generations are NOT implemented — they land
/// with the real client pool (Phase 1), where per-allocation ClientStatus
/// and deployment health become observable at all.
async fn operate(
    State(platform): State<SharedPlatform>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    ensure_tenant(&platform, &id, &principal)?;
    let (stage, allocation, tenant) = {
        let plat = platform.read().unwrap();
        let app = plat.apps.get(&id).ok_or_else(|| not_found("app"))?;
        (app.stage, app.allocation.clone(), app.tenant.clone())
    };
    let live = stage == Stage::Live;
    let cleanup_pending = allocation
        .as_ref()
        .is_some_and(|allocation| allocation.cleanup_pending);
    let cleanup_stopped = allocation
        .as_ref()
        .is_some_and(|allocation| allocation.cleanup_workload_stopped);
    let desired = if live && !cleanup_pending {
        "running"
    } else {
        "stopped"
    };
    let (observed, source) = if cleanup_pending && cleanup_stopped {
        // This is not a guess: cleanup_pending is written only after Nomad
        // confirmed the stop. Vault cleanup may remain, but the workload is
        // already known stopped.
        ("stopped".to_string(), "rollback-cleanup")
    } else if live && crate::hashi::Nomad::from_env().is_some() {
        let job_id = id.clone();
        let namespace = format!("tenant-{tenant}");
        let polled = tokio::task::spawn_blocking(move || {
            crate::hashi::Nomad::from_env()
                .expect("checked above; env does not change mid-request")
                .job_status(&job_id, &namespace)
        })
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("status poll panicked: {e}"),
            )
        })?
        .map_err(|e| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!("nomad is configured but the job status poll failed — refusing to claim an unobserved state: {e:#}"),
            )
        })?;
        (polled, "nomad")
    } else {
        (desired.to_string(), "simulated")
    };
    // Nomad job status is not an uptime or latency telemetry source. Until a
    // metrics backend is configured, return explicit unavailability instead
    // of polished constants. Health is true only when the orchestrator
    // actually reports a running job; the stored release record is not an
    // observation source.
    let observed_running = source == "nomad" && observed == "running";
    Ok(Json(json!({
        "stage": stage,
        "allocation": allocation,
        "desired_state": desired,
        "observed_state": observed,
        "status_source": source,
        "metrics": {
            "source": null,
            "available": false,
            "uptime_pct": null,
            "p95_ms": null,
            "healthy": observed_running,
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
    let _serial = serialize_app_mutation(&platform, &id).await;
    refuse_during_cleanup(&platform, &id)?;
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
