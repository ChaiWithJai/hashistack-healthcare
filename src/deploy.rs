//! Deploy service: promotes a gated app into the prod pool and renders its
//! Nomad job. It runs no compliance logic of its own — it demands a green
//! gate report and a co-signature, and refuses everything else. It never
//! generates code and never writes audit prose beyond its own deploy event.

use anyhow::{bail, Result};

use crate::gates::GateReport;
use crate::state::{now_unix, Allocation, AppRecord, Attestation, DataSource, Stage};

/// Current immutable client image, as baked by Packer (packer/client.pkr.hcl).
pub const CLIENT_IMAGE: &str = "client-v2026.07.1";
pub const REGION: &str = "nyc3";

const JOB_TEMPLATE: &str = include_str!("../nomad/templates/service-web.nomad.hcl.tmpl");

/// Promote sandbox → prod. The only path by which an app may ever see real
/// data, and it consumes the gate report it was handed as evidence.
///
/// TODO(#6): demo allocations are structs — nothing is submitted to Nomad and
/// `healthy` is a literal. Real deploys submit the rendered job, mirror
/// Nomad's desired/observed status axes, and split release from deploy.
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
    });
    app.attestation = Some(Attestation {
        cosigner: cosigner.trim().to_string(),
        gate_summary: report.summary(),
        reviewer_note: app.reviewer_note.clone(),
        at: now_unix(),
    });
    app.stage = Stage::Live;
    app.data_source = DataSource::Tenant(format!("tenant-{}", app.tenant));
    Ok(())
}

/// Roll back to the sandbox: the allocation is destroyed, not patched, and
/// the app returns to synthetic data. Immutability applies to exits too.
pub fn rollback(app: &mut AppRecord, synthetic_dataset: &str) -> Result<()> {
    if app.stage != Stage::Live {
        bail!("app {} has no live allocation to roll back", app.id);
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
