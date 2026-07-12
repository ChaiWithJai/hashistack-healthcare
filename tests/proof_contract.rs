use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use rust_proof_service::{app, proof_for_workload};
use tower::ServiceExt;

#[tokio::test]
async fn health_route_returns_ok() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn proof_route_accepts_workload_path() {
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/proof/deterministic_api")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn proof_contract_names_managed_default_and_rust_boundary() {
    let proof = proof_for_workload("deterministic_api".to_string());

    assert_eq!(proof.workload, "deterministic_api");
    assert!(proof.managed_default.contains("Supabase"));
    assert!(proof.rust_boundary.contains("replay"));
    assert!(proof.evidence.contains("meaningful failure"));
}
