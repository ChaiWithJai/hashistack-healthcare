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
pub mod deploy;
pub mod eject;
pub mod gates;
pub mod hashi;
pub mod ladder;
pub mod packs;
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
/// the id counter all survive a restart. Unset, identical to [`app`].
pub async fn app_from_env() -> anyhow::Result<Router> {
    let url = std::env::var("CONTROL_DB_URL")
        .ok()
        .filter(|u| !u.trim().is_empty());
    let Some(url) = url else {
        return Ok(app());
    };
    let mut platform = state::Platform::new(packs::builtin_packs());
    let pg = store::PgStore::connect(&url).await?;
    let (apps, ops, events) = pg.load(&mut platform).await?;
    platform.store = Some(std::sync::Arc::new(pg));
    tracing::info!(
        "control DB attached — restored {apps} apps, {ops} operations, {events} audit events"
    );
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
