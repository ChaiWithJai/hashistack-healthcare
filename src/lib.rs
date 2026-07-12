//! Clinician platform control plane — Phase 0 slice.
//!
//! Doctors describe tools in natural language and receive running,
//! HIPAA-scaffolded applications. The workflow is the fixed contract:
//!
//!   describe → generate → preview → iterate → gate → deploy → operate → audit
//!
//! Each module owns one verb and nothing else (the Tao, applied):
//! the agent does not deploy, the deployer does not audit, the auditor
//! does not schedule. This binary is one client-agnostic control-plane API;
//! the doctor UI served at `/` holds no privileges the API doesn't offer.
//!
//! The original proof-service contract (`/health`, `/proof/:workload`)
//! remains: this repo is still the proof that this backend slice deserves
//! owned Rust — the gate engine is the Rust-owned boundary.

pub mod agent;
pub mod api;
pub mod audit;
pub mod clerk;
pub mod deploy;
pub mod eject;
pub mod gates;
pub mod hashi;
pub mod identity;
pub mod ladder;
pub mod packs;
pub mod refusals;
pub mod state;
pub mod store;

use axum::{extract::Path, Json, Router};
use serde::Serialize;

#[derive(Serialize)]
pub struct Health {
    pub status: &'static str,
    pub service: &'static str,
}

#[derive(Serialize)]
pub struct Proof {
    pub workload: String,
    pub managed_default: &'static str,
    pub rust_boundary: &'static str,
    pub evidence: &'static str,
}

/// The full control plane with a fresh in-memory platform state.
pub fn app() -> Router {
    api::router()
}

/// The control plane as `main` boots it: with `CONTROL_DB_URL` set (#7),
/// connect to the Postgres control store, apply migrations idempotently,
/// and load the durable state back — apps, operations, audit stream, and
/// the id counter all survive a restart. With `AUDIT_FILE` set (#8), a
/// JSONL file sink joins the audit broker. Each durable sink must pass its
/// registration probe or the boot fails loudly. Neither set → identical to
/// [`app`] (dev mode: the in-memory fallback sink alone).
///
/// Identity (#10): `IDENTITIES_FILE` set → the declared registry, strict
/// bearer auth (missing/invalid tokens 401); unset → the embedded dev
/// registry with the audited dr-osei fallback. `SESSION_IDLE_SECS` turns on
/// idle auto-logoff for principal sessions in either mode. A registry that
/// fails to load fails the boot loudly.
pub async fn app_from_env() -> anyhow::Result<Router> {
    let registry = identity::Registry::from_env()?;
    let clerk = clerk::ClerkVerifier::from_env()?.map(std::sync::Arc::new);
    tracing::info!(
        "identity registry: {} principals from {} — dev fallback {}, session idle {}",
        registry.principal_count(),
        registry.source(),
        if registry.fallback().is_some() {
            "ON (audited)"
        } else {
            "off (strict 401s)"
        },
        registry
            .idle_secs()
            .map(|n| format!("{n}s"))
            .unwrap_or_else(|| "off".to_string()),
    );

    let db_url = std::env::var("CONTROL_DB_URL")
        .ok()
        .filter(|u| !u.trim().is_empty());
    let audit_file = std::env::var("AUDIT_FILE")
        .ok()
        .filter(|p| !p.trim().is_empty());
    if db_url.is_none() && audit_file.is_none() {
        let mut platform = state::Platform::new(packs::builtin_packs());
        platform.identity = std::sync::Arc::new(registry);
        platform.clerk = clerk;
        return Ok(api::router_with_state(std::sync::Arc::new(
            std::sync::RwLock::new(platform),
        )));
    }

    let mut platform = state::Platform::new(packs::builtin_packs());
    platform.identity = std::sync::Arc::new(registry);
    platform.clerk = clerk;
    let mut broker = audit::Broker::new();
    if let Some(url) = db_url {
        let pg = store::PgStore::connect(&url).await?;
        let (apps, ops, events) = pg.load(&mut platform).await?;
        let pg = std::sync::Arc::new(pg);
        platform.store = Some(pg.clone());
        broker
            .register(std::sync::Arc::new(store::PgSink::new(pg)))
            .await?;
        tracing::info!(
            "control DB attached — restored {apps} apps, {ops} operations, {events} audit events"
        );
    }
    if let Some(path) = audit_file {
        let sink = audit::FileSink::open(&path, platform.audit.head_seq())?;
        broker.register(std::sync::Arc::new(sink)).await?;
        tracing::info!("audit file sink attached — JSONL archive at {path}");
    }
    tracing::info!(
        "audit broker sinks: {:?} — load-bearing operations require a durable confirmation",
        broker.sink_names()
    );
    platform.broker = std::sync::Arc::new(broker);
    Ok(api::router_with_state(std::sync::Arc::new(
        std::sync::RwLock::new(platform),
    )))
}

pub(crate) async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: "clinician-platform-control-plane",
    })
}

pub(crate) async fn proof(Path(workload): Path<String>) -> Json<Proof> {
    Json(proof_for_workload(workload))
}

pub fn proof_for_workload(workload: String) -> Proof {
    Proof {
        workload,
        managed_default:
            "Use Supabase, Cloudflare, or an API first when they prove the workflow cheaply.",
        rust_boundary:
            "Own Rust where replay, correctness, async state, or traceable proof matters — \
             here: the gate engine, whose verdicts must be reproducible evidence.",
        evidence: "Add one test that proves the boundary under a meaningful failure.",
    }
}
