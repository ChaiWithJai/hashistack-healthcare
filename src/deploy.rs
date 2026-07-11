//! Deploy service: promotes a gated app into the prod pool and renders its
//! Nomad job. It runs no compliance logic of its own — it demands a green
//! gate report and a co-signature, and refuses everything else. It never
//! generates code and never writes audit prose beyond its own deploy event.

use anyhow::{bail, Context, Result};

use crate::gates::GateReport;
use crate::hashi;
use crate::identity::{Principal, Role};
use crate::state::{
    now_unix, valid_transition, Allocation, AppRecord, Attestation, DataSource, Stage,
};

/// Current immutable client image, as baked by Packer (packer/client.pkr.hcl).
pub const CLIENT_IMAGE: &str = "client-v2026.07.1";
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
/// a sha256 digest of the frozen gate report. `cosigner_claim` — the typed
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
/// TODO(#6): `healthy` is still a literal, and release≠deploy + generations
/// are NOT implemented — both land with the real client pool (Phase 1),
/// where per-allocation ClientStatus and deployment health exist to
/// observe. Stated in docs/investigations/0001's status matrix.
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
    if !report.green {
        let failing: Vec<String> = report.failing().iter().map(|r| r.title.clone()).collect();
        // Count what the message names: labeled stubs are satisfied-with-a-
        // caveat (#3), so `total - passed` would overcount and contradict
        // both the named list and the UI's meter (caught by the journey
        // profiler capturing this refusal verbatim).
        bail!(
            "deploy locked ({} failing): {}",
            failing.len(),
            failing.join("; ")
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

    let database = format!("tenant_{}_{}", app.tenant, app.id.replace('-', "_"));
    app.allocation = Some(Allocation {
        id: alloc_id,
        pool: "prod".to_string(),
        region: REGION.to_string(),
        image: CLIENT_IMAGE.to_string(),
        profile: "web".to_string(),
        database,
        vault_lease_id: None,
        vault_db_username: None,
        vault_lease_ttl_secs: None,
        credentials: "simulated: vault dynamic postgres creds, 1h TTL, auto-revoked (real lease issued in staging, #9)".to_string(),
        app_version: app.current_version,
        url: format!("{}.{}.app", app.id, app.tenant),
        healthy: true,
        deployed_at: now_unix(),
        nomad_eval_id: None,
        vault_transit_key: None,
    });
    app.attestation = Some(Attestation {
        cosigner,
        // #10: the cryptographic act — authenticated principal id + sha256
        // of the exact frozen report + timestamp, all on one record.
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
    app.data_source = DataSource::Tenant(format!("tenant-{}", app.tenant));
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
            pg_login_select1(&db_url, &lease.username, lease.password()).map_err(|e| {
                anyhow::anyhow!(
                    "vault issued lease {} but the credentials failed to authenticate: {e:#}",
                    lease.lease_id
                )
            })?;
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
            events.push(("vault.db_creds_issued".to_string(), detail));
        }
    }

    if let Some(nomad) = hashi::Nomad::from_env() {
        let job_hcl = render_job(app)?;
        nomad.ensure_namespace(&namespace)?;
        let eval_id = nomad.submit_job_hcl(&job_hcl)?;
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
pub fn staging_rollback(snapshot: &AppRecord) -> Result<Vec<(String, String)>> {
    let mut events = Vec::new();
    let namespace = format!("tenant-{}", snapshot.tenant);

    if let Some(nomad) = hashi::Nomad::from_env() {
        nomad.stop_job(&snapshot.id, &namespace)?;
        events.push((
            "nomad.job_stopped".to_string(),
            format!("job {} stopped in namespace {namespace}", snapshot.id),
        ));
    }

    if let Some(vault) = hashi::Vault::from_env() {
        let lease = snapshot.allocation.as_ref().and_then(|a| {
            a.vault_lease_id
                .as_deref()
                .zip(a.vault_db_username.as_deref())
        });
        if let Some((lease_id, username)) = lease {
            vault.revoke_lease(lease_id)?;
            if let Some(db_url) = staging_db_url() {
                if pg_login_select1(&db_url, username, "revocation-probe").is_ok() {
                    bail!(
                        "vault reported lease {lease_id} revoked but {username} still authenticates"
                    );
                }
                if pg_role_exists(&db_url, username)? {
                    bail!(
                        "vault reported lease {lease_id} revoked but role {username} still exists in pg_roles"
                    );
                }
            }
            events.push((
                "vault.lease_revoked".to_string(),
                format!(
                    "lease {lease_id} revoked — verified: {username} no longer authenticates \
                     and is dropped from pg_roles"
                ),
            ));
        }
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
    let Some(alloc) = &app.allocation else {
        bail!("app {} has no allocation to render", app.id);
    };
    let gate_summary = app
        .attestation
        .as_ref()
        .map(|a| a.gate_summary.replace('/', "-of-"))
        .unwrap_or_default();
    Ok(JOB_TEMPLATE
        .replace("{{app_id}}", &app.id)
        .replace("{{tenant}}", &app.tenant)
        .replace("{{region}}", &alloc.region)
        .replace("{{image}}", &alloc.image)
        .replace("{{database}}", &alloc.database)
        .replace("{{url}}", &alloc.url)
        .replace("{{gate_summary}}", &gate_summary)
        .replace("{{app_version}}", &alloc.app_version.to_string()))
}
