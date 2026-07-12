//! Deploy service: promotes a gated app into the prod pool and renders its
//! Nomad job. It runs no compliance logic of its own — it demands a green
//! gate report and a co-signature, and refuses everything else. It never
//! generates code and never writes audit prose beyond its own deploy event.

use anyhow::{bail, Context, Result};
use std::fmt;

use crate::gates::GateReport;
use crate::hashi;
use crate::identity::{validate_app_slug, validate_tenant_slug, Principal, Role};
use crate::state::{
    now_unix, valid_transition, Allocation, AppRecord, Attestation, DataSource, Stage,
};

fn validate_issued_lease<F, R>(lease_id: &str, validate: F, revoke: R) -> Result<()>
where
    F: FnOnce() -> Result<()>,
    R: FnOnce(&str) -> Result<()>,
{
    if let Err(validation_error) = validate() {
        if let Err(revoke_error) = revoke(lease_id) {
            bail!(
                "vault issued lease {lease_id} but its credentials failed validation ({validation_error:#}); compensation also failed to revoke it ({revoke_error:#})"
            );
        }
        bail!(
            "vault issued lease {lease_id} but its credentials failed validation ({validation_error:#}); lease revoked"
        );
    }
    Ok(())
}

#[derive(Debug)]
pub struct CleanupFailure {
    /// True means Nomad has confirmed the workload stop. Callers must retain
    /// cleanup intent instead of restoring a false live/running claim.
    pub workload_stopped: bool,
    cause: anyhow::Error,
}

impl CleanupFailure {
    pub fn injected(workload_stopped: bool, message: impl Into<String>) -> Self {
        Self {
            workload_stopped,
            cause: anyhow::anyhow!(message.into()),
        }
    }
}

impl fmt::Display for CleanupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#}", self.cause)
    }
}

impl std::error::Error for CleanupFailure {}

pub trait CleanupDriver: Send + Sync {
    fn rollback(
        &self,
        snapshot: &AppRecord,
    ) -> std::result::Result<Vec<(String, String)>, CleanupFailure>;
}

pub struct HashiCleanupDriver;

impl CleanupDriver for HashiCleanupDriver {
    fn rollback(
        &self,
        snapshot: &AppRecord,
    ) -> std::result::Result<Vec<(String, String)>, CleanupFailure> {
        staging_rollback(snapshot)
    }
}

fn cleanup_sequence<S, R, V>(
    already_stopped: bool,
    stop: S,
    revoke: R,
    verify: V,
) -> std::result::Result<bool, CleanupFailure>
where
    S: FnOnce() -> Result<()>,
    R: FnOnce() -> Result<()>,
    V: FnOnce() -> Result<()>,
{
    if !already_stopped {
        stop().map_err(|cause| CleanupFailure {
            workload_stopped: false,
            cause,
        })?;
    }
    revoke().map_err(|cause| CleanupFailure {
        workload_stopped: true,
        cause,
    })?;
    verify().map_err(|cause| CleanupFailure {
        workload_stopped: true,
        cause,
    })?;
    Ok(!already_stopped)
}

fn require_cleanup_clients(
    already_stopped: bool,
    has_nomad_handle: bool,
    nomad_available: bool,
    has_lease: bool,
    vault_available: bool,
    verification_db_available: bool,
) -> std::result::Result<(), CleanupFailure> {
    let unavailable = |message: &str| CleanupFailure {
        workload_stopped: already_stopped,
        cause: anyhow::anyhow!(message.to_string()),
    };
    if !already_stopped && has_nomad_handle && !nomad_available {
        return Err(unavailable("nomad cleanup client unavailable"));
    }
    if has_lease && !vault_available {
        return Err(unavailable("vault cleanup client unavailable"));
    }
    if has_lease && !verification_db_available {
        return Err(unavailable("staging database verification unavailable"));
    }
    Ok(())
}

/// Current immutable client image, as baked by Packer (packer/client.pkr.hcl).
pub const CLIENT_IMAGE: &str = "registry.internal/clinician-client@sha256:8f32c9b98d31f5ad0e0be9f92efc47942c74ec352af89c3254ddf967db2d19f7";
pub const REGION: &str = "nyc3";

const JOB_TEMPLATE: &str = include_str!("../nomad/templates/service-web.nomad.hcl.tmpl");
const POLICY_TEMPLATE: &str = include_str!("../vault/policies/tenant-app.hcl");

/// The Vault database-engine role staging-up.sh configures (#9). One shared
/// role today — per-tenant DB roles are a Phase 1 (cloud) item, tracked in
/// the runbook's honesty notes.
pub const DB_CREDS_ROLE: &str = "tenant-app";

/// Promote sandbox → prod. The only path by which an app may ever see real
/// data, and it consumes the gate report it was handed as evidence.
///
/// #10: the co-sign is the authenticated principal's own act. The caller
/// (api layer) already enforced tenancy (404) and the role capability (403,
/// audited); this function re-asserts both as defense in depth, then binds
/// the attestation to the principal id, their registered display name, and
/// a sha256 integrity digest of the frozen gate report. This is authenticated
/// attribution plus tamper evidence, not a public-key digital signature.
/// `cosigner_claim` — the typed
/// field the UI keeps — is only a display-name check: it must match the
/// principal's registered name exactly, or be omitted.
///
/// Staging (#2): with `NOMAD_ADDR` set, [`staging_promote`] then submits the
/// rendered job to a real Nomad dev agent and records the evaluation id; with
/// `VAULT_ADDR`/`VAULT_TOKEN` set it proves the tenant transit key. With
/// neither, the allocation stays a simulated struct exactly as before.
/// #6 (honest slice landed): the operate endpoint now reports Nomad's dual
/// status axes — `desired_state` from this record, `observed_state` polled
/// from the real job when `NOMAD_ADDR` is set (src/hashi.rs `job_status`).
/// Allocation health starts false and may only become true when a runtime
/// observation says the allocation is running. Release≠deploy + generations
/// remain Phase 1 work for the real client pool.
/// #9: with staging Vault + control DB present, [`staging_promote`] replaces
/// the placeholder credentials string below with a real database-engine
/// lease (lease id, username, TTL — proven to authenticate before it is
/// recorded). Without staging env the string stays, labeled as simulation.
pub fn promote(
    app: &mut AppRecord,
    report: &GateReport,
    principal: &Principal,
    cosigner_claim: Option<&str>,
    alloc_id: String,
    synthetic_demo: bool,
) -> Result<()> {
    if app.stage == Stage::Live {
        bail!(
            "app {} is already live — iterate and re-promote instead",
            app.id
        );
    }
    // The lifecycle transition table is the shared truth (#7): the same
    // pairs seed Postgres's app_valid_state, so an illegal transition is
    // structurally impossible in memory AND at the database.
    if !valid_transition(app.stage, Stage::Live) {
        bail!(
            "illegal lifecycle transition {}→{} for app {}",
            app.stage.as_str(),
            Stage::Live.as_str(),
            app.id
        );
    }
    if report.app_id != app.id || report.app_version != app.current_version {
        bail!("gate report is stale: it attests a different app or version");
    }
    let blockers = report.promotion_blockers(synthetic_demo);
    if !blockers.is_empty() {
        bail!(
            "deploy locked ({} blocking): {}",
            blockers.len(),
            blockers.join("; ")
        );
    }
    // Defense in depth (#10): the api layer already answered 404/403 with
    // audit events; a future caller that skips it still cannot cross these.
    if principal.role != Role::Clinician {
        bail!(
            "promotion requires a co-signature from the responsible clinician — role {} may not co-sign",
            principal.role.as_str()
        );
    }
    if principal.tenant != app.tenant {
        bail!(
            "principal tenant {} does not own app {}",
            principal.tenant,
            app.id
        );
    }
    // The typed cosigner field survives only as a display-name check: the
    // signature IS the authenticated principal; a claim naming anyone else
    // is refused, and omitting the field signs as the principal directly.
    let cosigner = match cosigner_claim.map(str::trim) {
        None => principal.name.clone(),
        Some(claim) if claim == principal.name => principal.name.clone(),
        Some(claim) => bail!(
            "co-signature {claim:?} does not match the authenticated clinician {:?} — \
             the co-sign is the principal's own act; omit the field or match the registered name",
            principal.name
        ),
    };

    let database = if synthetic_demo {
        "synthetic-demo-only".to_string()
    } else {
        format!("tenant_{}_{}", app.tenant, app.id.replace('-', "_"))
    };
    app.allocation = Some(Allocation {
        id: alloc_id,
        pool: if synthetic_demo {
            "synthetic-demo"
        } else {
            "prod"
        }
        .to_string(),
        region: REGION.to_string(),
        image: CLIENT_IMAGE.to_string(),
        profile: "web".to_string(),
        database,
        vault_lease_id: None,
        vault_db_username: None,
        vault_lease_ttl_secs: None,
        credentials: if synthetic_demo {
            "none: explicitly synthetic demo; tenant credentials are not issued".to_string()
        } else {
            "simulated: vault dynamic postgres creds, 1h TTL, auto-revoked (real lease issued in staging, #9)".to_string()
        },
        app_version: app.current_version,
        url: if synthetic_demo {
            format!("{}.synthetic-demo.local", app.id)
        } else {
            format!("{}.{}.app", app.id, app.tenant)
        },
        // A release record cannot manufacture an operational observation.
        // `operate` combines this conservative value with Nomad's observed
        // state, so pending/stopped/unknown jobs never report healthy.
        healthy: false,
        cleanup_pending: false,
        cleanup_workload_stopped: false,
        cleanup_error: None,
        deployed_at: now_unix(),
        nomad_eval_id: None,
        vault_transit_key: None,
    });
    app.attestation = Some(Attestation {
        cosigner,
        // #10: authenticated attribution + a tamper-evident sha256 digest
        // of the exact frozen report + timestamp, all on one record. The
        // digest is not represented as a public-key digital signature.
        principal: Some(principal.id.clone()),
        gate_summary: report.summary(),
        reviewer_note: app.reviewer_note.clone(),
        // F3: freeze the admitting report on the attestation verbatim — the
        // released app's compliance record embeds this, never a re-run.
        report: Some(report.clone()),
        report_digest: Some(crate::gates::report_digest(report)),
        at: now_unix(),
    });
    app.stage = Stage::Live;
    if !synthetic_demo {
        app.data_source = DataSource::Tenant(format!("tenant-{}", app.tenant));
    }
    Ok(())
}

/// The staging control DB URL (#7/#9) — the Postgres instance Vault's
/// database engine issues dynamic credentials against.
fn staging_db_url() -> Option<String> {
    std::env::var("CONTROL_DB_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Staging (#2, #9): the real-infrastructure half of a promotion. Called
/// after [`promote`] has produced the allocation; a no-op returning no
/// events when neither `NOMAD_ADDR` nor `VAULT_ADDR`+`VAULT_TOKEN` is set,
/// so every existing test and demo path is untouched.
///
/// Order matters: the Vault probe runs first because a tenant whose
/// encryption keys can't be proven must not have a job registered at all.
/// Returns (action, detail) pairs for the audit stream — evidence, not logs.
pub fn staging_promote(app: &mut AppRecord) -> Result<Vec<(String, String)>> {
    let mut events = Vec::new();
    let namespace = format!("tenant-{}", app.tenant);
    let mut issued_lease: Option<String> = None;

    // The production image is immutable and digest pinned. A local staging
    // client cannot pull registry.internal, so an explicit environment value
    // may select a locally built proof image. The chosen image is stored on
    // the allocation and therefore remains visible in state and audit evidence.
    if let Ok(image) = std::env::var("NOMAD_STAGING_IMAGE") {
        let image = image.trim();
        if image.is_empty() || image.chars().any(char::is_whitespace) {
            bail!("NOMAD_STAGING_IMAGE must be a non-empty image reference without whitespace");
        }
        if let Some(alloc) = app.allocation.as_mut() {
            alloc.image = image.to_string();
        }
        events.push((
            "nomad.staging_image_selected".to_string(),
            format!("local staging override selected image {image}"),
        ));
    }

    if let Some(vault) = hashi::Vault::from_env() {
        vault.transit_roundtrip(&namespace, &format!("promotion-probe-{}", app.id))?;
        if let Some(alloc) = app.allocation.as_mut() {
            alloc.vault_transit_key = Some(namespace.clone());
        }
        events.push((
            "vault.transit_verified".to_string(),
            format!("transit key {namespace}: encrypt/decrypt round-trip ok"),
        ));

        // #9: tenant first-promote mounts the tenant's ACL policy — the
        // rendered vault/policies/tenant-app.hcl, naming the exact transit
        // and database paths this tenant's allocations use. Honesty note:
        // dev-mode tokens are root, so the policy EXISTS and is read back
        // by the pressure test, but it is not yet the enforcing credential
        // — token-per-allocation enforcement is the Phase 1 cloud item.
        if vault.policy_read(&namespace)?.is_none() {
            let policy = POLICY_TEMPLATE.replace("TENANT", &app.tenant);
            vault.policy_write(&namespace, &policy)?;
            events.push((
                "vault.policy_mounted".to_string(),
                format!(
                    "acl policy {namespace} mounted at sys/policies/acl/{namespace}: \
                     transit/{{encrypt,decrypt}}/{namespace} + database/creds/{DB_CREDS_ROLE} \
                     (present, not yet token-enforced — dev root token; Phase 1)"
                ),
            ));
        }

        // #9: per-allocation dynamic database credentials, verified — the
        // issued user must actually authenticate against the staging
        // Postgres (SELECT 1 as that user) before the lease is recorded.
        // The password stays in this scope: never on the allocation, never
        // in the audit stream, never durable.
        if let Some(db_url) = staging_db_url() {
            let lease = vault.db_creds(DB_CREDS_ROLE)?;
            // The lease exists even when its credentials are unusable. Arm
            // compensation immediately after issuance, before any validation
            // or allocation mutation can fail.
            validate_issued_lease(
                &lease.lease_id,
                || pg_login_select1(&db_url, &lease.username, lease.password()),
                |lease_id| vault.revoke_lease(lease_id),
            )?;
            let detail = format!(
                "role {DB_CREDS_ROLE}: {} — verified: SELECT 1 as {} against the staging DB",
                lease.audit_detail(),
                lease.username
            );
            if let Some(alloc) = app.allocation.as_mut() {
                alloc.credentials = format!(
                    "vault database/creds/{DB_CREDS_ROLE}: lease {} as {}, ttl {}s, revoked on rollback",
                    lease.lease_id, lease.username, lease.ttl_secs
                );
                alloc.vault_lease_id = Some(lease.lease_id.clone());
                alloc.vault_db_username = Some(lease.username.clone());
                alloc.vault_lease_ttl_secs = Some(lease.ttl_secs);
            }
            issued_lease = Some(lease.lease_id.clone());
            events.push(("vault.db_creds_issued".to_string(), detail));
        }
    }

    if let Some(nomad) = hashi::Nomad::from_env() {
        let submitted = (|| -> Result<String> {
            let job_hcl = render_job(app)?;
            nomad.ensure_namespace(&namespace)?;
            nomad.submit_job_hcl(&job_hcl)
        })();
        let eval_id = match submitted {
            Ok(eval_id) => eval_id,
            Err(submit_error) => {
                // Packer-style compensation: preparation may have issued a
                // short-lived database credential before Nomad execution.
                // A refused submission must not orphan that lease.
                if let (Some(vault), Some(lease_id)) =
                    (hashi::Vault::from_env(), issued_lease.as_deref())
                {
                    if let Err(revoke_error) = vault.revoke_lease(lease_id) {
                        bail!(
                            "nomad submission failed ({submit_error:#}); compensation also failed to revoke vault lease {lease_id} ({revoke_error:#})"
                        );
                    }
                }
                return Err(
                    submit_error.context("nomad submission failed; issued vault lease revoked")
                );
            }
        };
        events.push((
            "nomad.job_submitted".to_string(),
            format!(
                "job {} registered in namespace {namespace}, evaluation {eval_id}",
                app.id
            ),
        ));
        if let Some(alloc) = app.allocation.as_mut() {
            alloc.nomad_eval_id = Some(eval_id);
        }
    }

    Ok(events)
}

/// Staging (#2, #9): the real-infrastructure half of a rollback, driven from
/// the pre-rollback snapshot (it still carries the allocation and its lease).
///
/// Order is the refusal semantics from #2: Nomad stops the job FIRST, and if
/// Nomad refuses, this returns Err before any lease is touched — a job that
/// is still running keeps its credentials. Only after the stop does Vault
/// revoke the allocation's database lease, and revocation is then PROVEN,
/// not claimed: the issued user must fail to authenticate and must be gone
/// from `pg_roles` (the engine's revocation drops the role).
///
/// Honesty note on the proof shape: the lease password is never retained
/// (the control plane persists no secrets, and a kill -9 restart sits
/// between promote and rollback in the pressure test), so the platform-side
/// proof is login-failure as the issued user plus the role's absence from
/// `pg_roles`. The pressure test additionally holds a sibling lease's
/// password end-to-end and asserts the literal authenticate-then-fail.
pub fn staging_rollback(
    snapshot: &AppRecord,
) -> std::result::Result<Vec<(String, String)>, CleanupFailure> {
    let mut events = Vec::new();
    let namespace = format!("tenant-{}", snapshot.tenant);
    let already_stopped = snapshot
        .allocation
        .as_ref()
        .is_some_and(|a| a.cleanup_workload_stopped);
    let nomad = hashi::Nomad::from_env();
    let vault = hashi::Vault::from_env();
    let lease = snapshot.allocation.as_ref().and_then(|a| {
        a.vault_lease_id
            .as_deref()
            .zip(a.vault_db_username.as_deref())
    });
    let db_url = staging_db_url();
    require_cleanup_clients(
        already_stopped,
        snapshot
            .allocation
            .as_ref()
            .and_then(|a| a.nomad_eval_id.as_ref())
            .is_some(),
        nomad.is_some(),
        lease.is_some(),
        vault.is_some(),
        db_url.is_some(),
    )?;

    let stopped_now = cleanup_sequence(
        already_stopped,
        || match &nomad {
            Some(nomad) => nomad.stop_job(&snapshot.id, &namespace),
            None if snapshot
                .allocation
                .as_ref()
                .and_then(|a| a.nomad_eval_id.as_ref())
                .is_some() =>
            {
                bail!("nomad cleanup client unavailable")
            }
            None => Ok(()),
        },
        || match (&vault, lease) {
            (Some(vault), Some((lease_id, _))) => vault.revoke_lease(lease_id),
            (None, Some(_)) => bail!("vault cleanup client unavailable"),
            (_, None) => Ok(()),
        },
        || match (db_url.as_deref(), lease) {
            (Some(db_url), Some((lease_id, username))) => {
                if pg_login_select1(db_url, username, "revocation-probe").is_ok() {
                    bail!(
                        "vault reported lease {lease_id} revoked but {username} still authenticates"
                    );
                }
                if pg_role_exists(db_url, username)? {
                    bail!(
                        "vault reported lease {lease_id} revoked but role {username} still exists in pg_roles"
                    );
                }
                Ok(())
            }
            (None, Some(_)) => bail!("staging database verification unavailable"),
            (_, None) => Ok(()),
        },
    )?;

    if nomad.is_some() {
        events.push((
            if stopped_now {
                "nomad.job_stopped"
            } else {
                "nomad.job_stop_already_confirmed"
            }
            .to_string(),
            format!(
                "job {} {} in namespace {namespace}",
                snapshot.id,
                if stopped_now {
                    "stopped"
                } else {
                    "was already stopped; resumed cleanup"
                }
            ),
        ));
    }
    if let Some((lease_id, username)) = lease {
        events.push((
            "vault.lease_revoked".to_string(),
            format!(
                "lease {lease_id} revoked — verified: {username} no longer authenticates \
                 and is dropped from pg_roles"
            ),
        ));
    }
    Ok(events)
}

// ---------- staging Postgres evidence probes (#9) ----------

/// One-shot `SELECT 1` AS the given user against the staging DB — the
/// evidence that an issued credential authenticates (and, negated, that a
/// revoked one no longer does). Runs on a dedicated thread with its own
/// single-threaded runtime so it is callable from sync code regardless of
/// the caller's async context; bounded by a connect timeout.
fn pg_login_select1(db_url: &str, user: &str, password: &str) -> Result<()> {
    let mut cfg: tokio_postgres::Config = db_url
        .parse()
        .with_context(|| format!("parsing staging DB url {db_url}"))?;
    cfg.user(user);
    cfg.password(password);
    let n = pg_one_shot(cfg, "SELECT 1".to_string(), Vec::new())?;
    if n != 1 {
        bail!("SELECT 1 as {user} returned {n}");
    }
    Ok(())
}

/// Does a role exist in `pg_roles`? Asked as the staging superuser (the
/// `CONTROL_DB_URL` credentials) — the read-back half of the revocation
/// proof: the database engine's revocation drops the issued role.
fn pg_role_exists(db_url: &str, role: &str) -> Result<bool> {
    let cfg: tokio_postgres::Config = db_url
        .parse()
        .with_context(|| format!("parsing staging DB url {db_url}"))?;
    let n = pg_one_shot(
        cfg,
        "SELECT count(*)::bigint FROM pg_roles WHERE rolname = $1".to_string(),
        vec![role.to_string()],
    )?;
    Ok(n > 0)
}

/// Run one single-row/single-column i64-ish query on a fresh connection and
/// tear it down. A dedicated thread + current-thread runtime keeps this
/// usable from synchronous deploy code (which may or may not be inside the
/// server's runtime) without `block_on` re-entrancy.
fn pg_one_shot(mut cfg: tokio_postgres::Config, sql: String, params: Vec<String>) -> Result<i64> {
    cfg.connect_timeout(std::time::Duration::from_secs(5));
    let handle = std::thread::spawn(move || -> Result<i64> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building one-shot pg runtime")?;
        rt.block_on(async move {
            let (client, connection) = cfg
                .connect(tokio_postgres::NoTls)
                .await
                .context("connecting")?;
            let conn = tokio::spawn(connection);
            let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                .iter()
                .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();
            let row = client.query_one(&sql, &refs).await.context("querying")?;
            let value: i64 = match row.try_get::<_, i64>(0) {
                Ok(v) => v,
                Err(_) => i64::from(row.try_get::<_, i32>(0).context("reading result column")?),
            };
            drop(client);
            conn.abort();
            Ok(value)
        })
    });
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("one-shot pg thread panicked"))?
}

/// Roll back to the sandbox: the allocation is destroyed, not patched, and
/// the app returns to synthetic data. Immutability applies to exits too.
pub fn rollback(app: &mut AppRecord, synthetic_dataset: &str) -> Result<()> {
    if app.stage != Stage::Live {
        bail!("app {} has no live allocation to roll back", app.id);
    }
    // Same shared transition table as promote (#7).
    if !valid_transition(app.stage, Stage::Sandbox) {
        bail!(
            "illegal lifecycle transition {}→{} for app {}",
            app.stage.as_str(),
            Stage::Sandbox.as_str(),
            app.id
        );
    }
    app.allocation = None;
    app.stage = Stage::Sandbox;
    app.data_source = DataSource::Synthetic(synthetic_dataset.to_string());
    Ok(())
}

/// Render the Nomad job for a live allocation — also the portability export:
/// no hostage code means the doctor can read exactly what runs.
pub fn render_job(app: &AppRecord) -> Result<String> {
    validate_tenant_slug(&app.tenant).context("unsafe tenant for Nomad job")?;
    validate_app_slug(&app.id).context("unsafe app id for Nomad job")?;
    let Some(alloc) = &app.allocation else {
        bail!("app {} has no allocation to render", app.id);
    };
    let gate_summary = app
        .attestation
        .as_ref()
        .map(|a| a.gate_summary.replace('/', "-of-"))
        .unwrap_or_default();
    let rendered = JOB_TEMPLATE
        .replace("{{app_id}}", &app.id)
        .replace("{{tenant}}", &app.tenant)
        .replace("{{region}}", &alloc.region)
        .replace("{{image}}", &alloc.image)
        .replace("{{pool}}", &alloc.pool)
        .replace("{{database}}", &alloc.database)
        .replace("{{url}}", &alloc.url)
        .replace("{{gate_summary}}", &gate_summary)
        .replace("{{app_version}}", &alloc.app_version.to_string());
    for token in [
        "{{app_id}}",
        "{{tenant}}",
        "{{region}}",
        "{{image}}",
        "{{database}}",
        "{{url}}",
        "{{gate_summary}}",
        "{{app_version}}",
        "{{pool}}",
    ] {
        if rendered.contains(token) {
            bail!("Nomad job rendering left unresolved token {token}");
        }
    }
    let rendered = if alloc.pool == "synthetic-demo" {
        strip_tenant_secrets(rendered)?
    } else {
        rendered
    };
    Ok(rendered)
}

fn strip_tenant_secrets(rendered: String) -> Result<String> {
    const START: &str = "      # BEGIN_TENANT_SECRETS — removed entirely for synthetic-demo jobs.";
    const END: &str = "      # END_TENANT_SECRETS";
    let start = rendered
        .find(START)
        .context("tenant-secret start marker missing")?;
    let end = rendered
        .find(END)
        .context("tenant-secret end marker missing")?
        + END.len();
    let mut safe = rendered;
    safe.replace_range(
        start..end,
        "      # synthetic demo: no Vault policy and no database credentials\n",
    );
    Ok(safe)
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn production_job_template_has_a_bounded_unprivileged_container_contract() {
        for required in [
            "user         = \"65532\"",
            "readonly_rootfs  = true",
            "cap_drop         = [\"ALL\"]",
            "no-new-privileges:true",
            "pids_limit",
            "memory_max",
            "ephemeral_disk",
            "auto_revert",
            "perms       = \"0400\"",
        ] {
            assert!(JOB_TEMPLATE.contains(required), "missing {required}");
        }
        assert!(
            CLIENT_IMAGE.contains("@sha256:"),
            "image must be digest-pinned"
        );
        for forbidden in [
            "privileged = true",
            "/var/run/docker.sock",
            "network_mode = \"host\"",
        ] {
            assert!(
                !JOB_TEMPLATE.contains(forbidden),
                "forbidden workload setting: {forbidden}"
            );
        }
    }

    #[test]
    fn invalid_issued_credentials_revoke_the_lease_exactly_once() {
        let revocations = AtomicUsize::new(0);
        let err = validate_issued_lease(
            "database/creds/tenant-app/lease-1",
            || bail!("SELECT 1 authentication failed"),
            |lease_id| {
                assert_eq!(lease_id, "database/creds/tenant-app/lease-1");
                revocations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(revocations.load(Ordering::SeqCst), 1);
        assert!(err.to_string().contains("lease revoked"));
    }

    #[test]
    fn invalid_credentials_report_failed_compensation() {
        let err = validate_issued_lease(
            "lease-2",
            || bail!("credential rejected"),
            |_| bail!("vault unavailable"),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("credential rejected"), "{message}");
        assert!(message.contains("vault unavailable"), "{message}");
        assert!(message.contains("failed to revoke"), "{message}");
    }

    #[test]
    fn stopped_workload_with_failed_revoke_is_cleanup_pending() {
        let stops = AtomicUsize::new(0);
        let err = cleanup_sequence(
            false,
            || {
                stops.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            || bail!("vault unavailable"),
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(err.workload_stopped);
        assert!(err.to_string().contains("vault unavailable"));
    }

    #[test]
    fn cleanup_retry_skips_the_already_confirmed_stop() {
        let stops = AtomicUsize::new(0);
        let stopped_now = cleanup_sequence(
            true,
            || {
                stops.fetch_add(1, Ordering::SeqCst);
                bail!("must not stop twice")
            },
            || Ok(()),
            || Ok(()),
        )
        .unwrap();
        assert!(!stopped_now);
        assert_eq!(stops.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stop_failure_never_claims_workload_stopped() {
        let err = cleanup_sequence(false, || bail!("nomad refused stop"), || Ok(()), || Ok(()))
            .unwrap_err();
        assert!(!err.workload_stopped);
        assert!(err.to_string().contains("nomad refused stop"));
    }

    #[test]
    fn persisted_cleanup_handles_require_every_verifier() {
        let missing_nomad =
            require_cleanup_clients(false, true, false, false, false, false).unwrap_err();
        assert!(!missing_nomad.workload_stopped);
        assert!(missing_nomad.to_string().contains("nomad"));

        let missing_vault =
            require_cleanup_clients(true, true, false, true, false, true).unwrap_err();
        assert!(missing_vault.workload_stopped);
        assert!(missing_vault.to_string().contains("vault"));

        let missing_db = require_cleanup_clients(true, true, false, true, true, false).unwrap_err();
        assert!(missing_db.workload_stopped);
        assert!(missing_db.to_string().contains("database verification"));
    }
}
