//! ADVERSARIAL FIXTURE (issue #3) — a deliberately broken scaffold.
//!
//! This file is NOT a shipped pack and is never compiled or registered: it
//! is embedded as a string by tests/evidence_contract.rs to prove that each
//! evidence gate fails on the defect aimed at it. If a gate ever passes
//! this source, the gate is lying. Defects, by gate:
//!
//! - audit-log:      a data route registered AFTER the audit layer
//! - phi-encryption: PHI fields with no disposition marker, plus a struct
//!                   claiming vault-transit with no encrypt call site
//! - ai-allowlist:   a rogue AI endpoint outside the pack allowlist
//! - synthetic-only: no SYNTHETIC-notice boot guard anywhere

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

struct Visit {
    // DEFECT (phi-encryption): PHI-marked fields but the struct declares no
    // disposition marker at all — the fields bypass encryption entirely.
    patient_name: String, // phi: patient name
    diagnosis: String,    // phi: diagnosis code
}

struct Claim {
    // phi-encryption: vault-transit
    // DEFECT (phi-encryption): the disposition above claims real transit
    // encryption, but no encrypt_field call site exists in this file.
    member_id: String, // phi: insurance member id
}

async fn summarize(visit: &Visit) -> String {
    // DEFECT (ai-allowlist): rogue endpoint outside the pack's signed
    // network allowlist — the exact PHI-leak shape the gate exists for.
    http_post(
        "https://api.openai.com/v1/chat/completions",
        &visit.diagnosis,
    )
    .await
}

fn app() -> Router {
    Router::new()
        .route("/", get(home))
        .route("/visits", post(record_visit))
        .layer(middleware::from_fn(audit_jsonl))
        // DEFECT (audit-log): registered AFTER the audit layer, so the
        // middleware never wraps it — an un-audited data route.
        .route("/admin/export-everything", get(export_everything))
}

fn load_dataset(json: &str) -> Dataset {
    // DEFECT (synthetic-only): parses whatever it is handed — no notice
    // check, no refusal path. Real data would boot without complaint.
    serde_json::from_str(json).unwrap()
}
