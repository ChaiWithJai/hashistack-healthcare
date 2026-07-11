//! Deploy service: promotes a gated app into the prod pool and renders its
//! Nomad job. It runs no compliance logic of its own — it demands a green
//! gate report and a co-signature, and refuses everything else. It never
//! generates code and never writes audit prose beyond its own deploy event.

use anyhow::{bail, Result};

use crate::gates::GateReport;
use crate::hashi;
use crate::state::{
    now_unix, valid_transition, Allocation, AppRecord, Attestation, DataSource, Stage,
};

/// Current immutable client image, as baked by Packer (packer/client.pkr.hcl).
pub const CLIENT_IMAGE: &str = "client-v2026.07.1";
pub const REGION: &str = "nyc3";

const JOB_TEMPLATE: &str = include_str!("../nomad/templates/service-web.nomad.hcl.tmpl");

/// Promote sandbox → prod. The only path by which an app may ever see real
/// data, and it consumes the gate report it was handed as evidence.
///
/// Staging (#2): with `NOMAD_ADDR` set, [`staging_promote`] then submits the
/// rendered job to a real Nomad dev agent and records the evaluation id; with
/// `VAULT_ADDR`/`VAULT_TOKEN` set it proves the tenant transit key. With
/// neither, the allocation stays a simulated struct exactly as before.
/// TODO(#6): `healthy` is still a literal. Real deploys mirror Nomad's
/// desired/observed status axes and split release from deploy.
/// TODO(#9): the credentials string below becomes a real Vault database-
/// engine lease; transit keys mount per tenant at onboarding.
pub fn promote(
    app: &mut AppRecord,
    report: &GateReport,
    cosigner: &str,
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
        bail!(
            "deploy locked ({} failing): {}",
            report.total - report.passed,
            failing.join("; ")
        );
    }
    if cosigner.trim().is_empty() {
        bail!("promotion requires a co-signature from the responsible clinician");
    }

    let database = format!("tenant_{}_{}", app.tenant, app.id.replace('-', "_"));
    app.allocation = Some(Allocation {
        id: alloc_id,
        pool: "prod".to_string(),
        region: REGION.to_string(),
        image: CLIENT_IMAGE.to_string(),
        profile: "web".to_string(),
        database,
        credentials: "vault: dynamic postgres creds, 1h TTL, auto-revoked".to_string(),
        app_version: app.current_version,
        url: format!("{}.{}.app", app.id, app.tenant),
        healthy: true,
        deployed_at: now_unix(),
        nomad_eval_id: None,
        vault_transit_key: None,
    });
    app.attestation = Some(Attestation {
        cosigner: cosigner.trim().to_string(),
        gate_summary: report.summary(),
        reviewer_note: app.reviewer_note.clone(),
        // F3: freeze the admitting report on the attestation verbatim — the
        // released app's compliance record embeds this, never a re-run.
        report: Some(report.clone()),
        at: now_unix(),
    });
    app.stage = Stage::Live;
    app.data_source = DataSource::Tenant(format!("tenant-{}", app.tenant));
    Ok(())
}

/// Staging (#2): the real-infrastructure half of a promotion. Called after
/// [`promote`] has produced the allocation; a no-op returning no events when
/// neither `NOMAD_ADDR` nor `VAULT_ADDR`+`VAULT_TOKEN` is set, so every
/// existing test and demo path is untouched.
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

/// Staging (#2): stop the real Nomad job on rollback (stopped, not purged —
/// the dead job stays inspectable, mirroring the append-only audit posture).
/// No-op without `NOMAD_ADDR`.
pub fn staging_rollback(app_id: &str, tenant: &str) -> Result<Vec<(String, String)>> {
    let mut events = Vec::new();
    if let Some(nomad) = hashi::Nomad::from_env() {
        let namespace = format!("tenant-{tenant}");
        nomad.stop_job(app_id, &namespace)?;
        events.push((
            "nomad.job_stopped".to_string(),
            format!("job {app_id} stopped in namespace {namespace}"),
        ));
    }
    Ok(events)
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
