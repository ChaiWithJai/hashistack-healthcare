//! Runnable synthetic insurance eligibility and referral queue.
//! Security boundary: preview data is in memory only. Real payer API calls,
//! OIDC/RBAC, Vault-backed field encryption, and durable audit storage are TODOs.

use axum::{
    extract::{Form, Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const EMBEDDED: &str = include_str!("../../synthetic/insurance-demo.json");

#[derive(Clone, Deserialize)]
struct Dataset {
    dataset: String,
    notice: String,
    cases: Vec<Case>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Case {
    id: String,
    patient: String, // phi: patient name; synthetic preview only
    visit: String,
    payer: String,
    member_id: String, // phi: insurance member id
    referral_required: bool,
    referral_on_file: bool,
    status: String,
}

enum AuditSink {
    Stdout,
    #[cfg_attr(not(test), allow(dead_code))]
    Memory(Mutex<Vec<String>>),
}
impl AuditSink {
    fn write(&self, s: String) {
        match self {
            Self::Stdout => println!("{s}"),
            Self::Memory(v) => v.lock().unwrap().push(s),
        }
    }
}
struct Inner {
    dataset: String,
    cases: Mutex<Vec<Case>>,
    audit: AuditSink,
}
#[derive(Clone)]
struct AppState(Arc<Inner>);

impl AppState {
    fn load(raw: &str, audit: AuditSink) -> Result<Self, String> {
        let d: Dataset = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if !d.notice.contains("SYNTHETIC") {
            return Err(format!(
                "refusing to boot: dataset {:?} is not marked SYNTHETIC",
                d.dataset
            ));
        }
        Ok(Self(Arc::new(Inner {
            dataset: d.dataset,
            cases: Mutex::new(d.cases),
            audit,
        })))
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
async fn audit(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let res = next.run(req).await;
    s.0.audit.write(serde_json::json!({"at":now(),"event":"http_request","method":method,"path":path,"status":res.status().as_u16(),"control":"audit-log-preview","durable":false}).to_string());
    res
}
fn page(title: &str, body: String) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>{title}</title><style>body{{font:16px system-ui;max-width:1050px;margin:auto;padding:2rem;background:#f8fafc;color:#172033}}nav,a{{color:#165d50}}table{{width:100%;border-collapse:collapse;background:white}}th,td{{padding:.7rem;border:1px solid #ccd5df;text-align:left}}.warn{{background:#fff3cd;padding:1rem}}button{{padding:.5rem;background:#165d50;color:white;border:0}}:focus{{outline:3px solid #f59e0b}}</style></head><body><nav><a href="/">Eligibility queue</a> · <a href="/api/cases">JSON</a></nav><h1>{title}</h1>{body}<footer><p><strong>Synthetic preview.</strong> No payer network is contacted. OIDC/RBAC, Vault encryption, durable audit retention, and approved endpoint enforcement are required before real use.</p></footer></body></html>"#
    ))
}

async fn health(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","dataset":s.0.dataset,"mode":"synthetic"}))
}
async fn home(State(s): State<AppState>) -> Html<String> {
    let rows=s.0.cases.lock().unwrap().iter().map(|c| format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><form method=post action=/cases/{}/verify><button>Run synthetic check</button></form></td></tr>",c.id,c.patient,c.visit,c.payer,c.status,if c.referral_required {if c.referral_on_file{"on file"}else{"missing"}}else{"not required"},c.id)).collect::<String>();
    page("Insurance eligibility queue",format!("<p class=warn>Human review required. Results are simulated and must not be used for coverage decisions.</p><table><caption>Upcoming visits needing front-desk review</caption><thead><tr><th>Case</th><th>Patient</th><th>Visit</th><th>Payer</th><th>Status</th><th>Referral</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table>"))
}
async fn cases(State(s): State<AppState>) -> Json<Vec<Case>> {
    Json(s.0.cases.lock().unwrap().clone())
}
async fn verify(Path(id): Path<String>, State(s): State<AppState>) -> impl IntoResponse {
    let mut cases = s.0.cases.lock().unwrap();
    let Some(c) = cases.iter_mut().find(|c| c.id == id) else {
        return (StatusCode::NOT_FOUND, "case not found").into_response();
    };
    c.status = if c.referral_required && !c.referral_on_file {
        "needs-referral"
    } else {
        "verified"
    }
    .into();
    s.0.audit.write(serde_json::json!({"at":now(),"event":"synthetic_eligibility_checked","case_id":id,"outcome":c.status,"human_review_required":true}).to_string());
    (StatusCode::SEE_OTHER, [(axum::http::header::LOCATION, "/")]).into_response()
}
#[derive(Deserialize)]
struct Referral {
    on_file: bool,
}
async fn referral(
    Path(id): Path<String>,
    State(s): State<AppState>,
    Form(f): Form<Referral>,
) -> impl IntoResponse {
    let mut cases = s.0.cases.lock().unwrap();
    let Some(c) = cases.iter_mut().find(|c| c.id == id) else {
        return StatusCode::NOT_FOUND;
    };
    c.referral_on_file = f.on_file;
    c.status = "pending".into();
    StatusCode::NO_CONTENT
}
fn router(s: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/api/cases", get(cases))
        .route("/cases/:id/verify", post(verify))
        .route("/cases/:id/referral", post(referral))
        .layer(middleware::from_fn_with_state(s.clone(), audit))
        .with_state(s)
}

#[tokio::main]
async fn main() {
    let raw = std::env::var("SYNTHETIC_DATA")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| EMBEDDED.into());
    let state = AppState::load(&raw, AuditSink::Stdout).expect("synthetic dataset required");
    let addr = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("insurance verification preview listening on http://{addr}");
    axum::serve(listener, router(state)).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    fn state() -> AppState {
        AppState::load(EMBEDDED, AuditSink::Memory(Mutex::new(vec![]))).unwrap()
    }
    #[tokio::test]
    async fn health_and_accessible_html() {
        let app = router(state());
        let r = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(r.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("text/html"));
    }
    #[test]
    fn rejects_non_synthetic() {
        let bad = EMBEDDED.replace("SYNTHETIC DATA", "DEMO DATA");
        assert!(AppState::load(&bad, AuditSink::Stdout).is_err());
    }
    #[tokio::test]
    async fn verification_routes_missing_referral() {
        let s = state();
        let app = router(s.clone());
        let r = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/cases/iv-102/verify")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        assert_eq!(s.0.cases.lock().unwrap()[1].status, "needs-referral");
        assert!(!match &s.0.audit {
            AuditSink::Memory(v) => v.lock().unwrap().is_empty(),
            _ => true,
        });
    }
}
