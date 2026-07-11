use axum::{
    extract::Path,
    routing::{get, post},
    Json, Router,
};
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

pub fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/proof/:workload", post(proof))
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: "rust-proof-service",
    })
}

async fn proof(Path(workload): Path<String>) -> Json<Proof> {
    Json(proof_for_workload(workload))
}

pub fn proof_for_workload(workload: String) -> Proof {
    Proof {
        workload,
        managed_default:
            "Use Supabase, Cloudflare, or an API first when they prove the workflow cheaply.",
        rust_boundary:
            "Own Rust where replay, correctness, async state, or traceable proof matters.",
        evidence: "Add one test that proves the boundary under a meaningful failure.",
    }
}
