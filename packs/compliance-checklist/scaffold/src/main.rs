//! Runnable safeguards, training, and evidence checklist over synthetic data.
//! This is an operational aid, not a certification of HIPAA compliance.
//! Evidence is metadata-only and in memory. OIDC/RBAC, malware scanning,
//! encrypted object storage, Vault keys, and durable audit retention are TODOs.

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
const EMBEDDED: &str = include_str!("../../synthetic/compliance-demo.json");

#[derive(Clone, Deserialize)]
struct Dataset {
    dataset: String,
    notice: String,
    clinic: String,
    safeguards: Vec<Safeguard>,
    training: Vec<Training>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Safeguard {
    id: String,
    title: String,
    owner: String,
    due: String,
    recurrence: String,
    complete: bool,
    evidence: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Training {
    id: String,
    staff: String,
    course: String,
    due: String,
    complete: bool,
}
#[derive(Clone, Serialize)]
struct Evidence {
    id: String,
    safeguard_id: String,
    label: String,
    encrypted_at_rest: bool,
    malware_scanned: bool,
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
    clinic: String,
    safeguards: Mutex<Vec<Safeguard>>,
    training: Mutex<Vec<Training>>,
    evidence: Mutex<Vec<Evidence>>,
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
            clinic: d.clinic,
            safeguards: Mutex::new(d.safeguards),
            training: Mutex::new(d.training),
            evidence: Mutex::new(vec![]),
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
    let r = next.run(req).await;
    s.0.audit.write(serde_json::json!({"at":now(),"event":"http_request","method":method,"path":path,"status":r.status().as_u16(),"control":"audit-log-preview","durable":false}).to_string());
    r
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn page(title: &str, body: String) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content="width=device-width"><title>{title}</title><style>body{{font:16px system-ui;max-width:1050px;margin:auto;padding:2rem;background:#f7f5f0;color:#20231f}}a{{color:#376341}}section{{background:white;border:1px solid #cad2c8;padding:1rem;margin:1rem 0}}table{{width:100%;border-collapse:collapse}}th,td{{padding:.65rem;border-bottom:1px solid #ddd;text-align:left}}button{{padding:.45rem;background:#376341;color:white;border:0}}.warn{{background:#fff3cd;padding:1rem}}:focus{{outline:3px solid #e19b28}}</style></head><body><nav><a href="/">Checklist</a> · <a href="/api/summary">JSON summary</a></nav><h1>{title}</h1>{body}<footer><p><strong>Synthetic preview; not a compliance determination.</strong> Real use requires authenticated roles, encrypted/scanned evidence storage, and durable audit retention.</p></footer></body></html>"#
    ))
}
async fn health(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","dataset":s.0.dataset,"mode":"synthetic"}))
}
async fn home(State(s): State<AppState>) -> Html<String> {
    let safeguards=s.0.safeguards.lock().unwrap().iter().map(|x|format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><form method=post action=/safeguards/{}/toggle><button>{}</button></form></td></tr>",esc(&x.title),esc(&x.owner),x.due,esc(&x.recurrence),x.evidence.as_deref().unwrap_or("none"),x.id,if x.complete{"Reopen"}else{"Complete"})).collect::<String>();
    let training=s.0.training.lock().unwrap().iter().map(|x|format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><form method=post action=/training/{}/toggle><button>{}</button></form></td></tr>",esc(&x.staff),esc(&x.course),x.due,if x.complete{"complete"}else{"due"},x.id,if x.complete{"Reopen"}else{"Complete"})).collect::<String>();
    page(&format!("{} compliance workspace",esc(&s.0.clinic)),format!("<p class=warn>Track work and evidence here; a checked box does not prove compliance.</p><section><h2>Safeguards</h2><table><thead><tr><th>Safeguard</th><th>Owner</th><th>Due</th><th>Repeats</th><th>Evidence</th><th>Action</th></tr></thead><tbody>{safeguards}</tbody></table></section><section><h2>Training</h2><table><thead><tr><th>Staff</th><th>Course</th><th>Due</th><th>Status</th><th>Action</th></tr></thead><tbody>{training}</tbody></table></section>"))
}
async fn summary(State(s): State<AppState>) -> Json<serde_json::Value> {
    let sg = s.0.safeguards.lock().unwrap();
    let tr = s.0.training.lock().unwrap();
    Json(
        serde_json::json!({"clinic":s.0.clinic,"safeguards":{"total":sg.len(),"complete":sg.iter().filter(|x|x.complete).count()},"training":{"total":tr.len(),"complete":tr.iter().filter(|x|x.complete).count()},"evidence_records":s.0.evidence.lock().unwrap().len(),"disclaimer":"tracker status is not a compliance determination"}),
    )
}
async fn toggle_safeguard(Path(id): Path<String>, State(s): State<AppState>) -> impl IntoResponse {
    let mut xs = s.0.safeguards.lock().unwrap();
    let Some(x) = xs.iter_mut().find(|x| x.id == id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    x.complete = !x.complete;
    s.0.audit.write(serde_json::json!({"at":now(),"event":"safeguard_status_changed","safeguard_id":id,"complete":x.complete}).to_string());
    (StatusCode::SEE_OTHER, [(axum::http::header::LOCATION, "/")]).into_response()
}
async fn toggle_training(Path(id): Path<String>, State(s): State<AppState>) -> impl IntoResponse {
    let mut xs = s.0.training.lock().unwrap();
    let Some(x) = xs.iter_mut().find(|x| x.id == id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    x.complete = !x.complete;
    s.0.audit.write(serde_json::json!({"at":now(),"event":"training_status_changed","training_id":id,"complete":x.complete}).to_string());
    (StatusCode::SEE_OTHER, [(axum::http::header::LOCATION, "/")]).into_response()
}
#[derive(Deserialize)]
struct EvidenceForm {
    label: String,
}
async fn add_evidence(
    Path(id): Path<String>,
    State(s): State<AppState>,
    Form(f): Form<EvidenceForm>,
) -> impl IntoResponse {
    if !s.0.safeguards.lock().unwrap().iter().any(|x| x.id == id) {
        return StatusCode::NOT_FOUND;
    }
    let eid = format!("ev-{}-{}", id, now());
    s.0.evidence.lock().unwrap().push(Evidence {
        id: eid.clone(),
        safeguard_id: id.clone(),
        label: f.label,
        encrypted_at_rest: false,
        malware_scanned: false,
    });
    if let Some(x) =
        s.0.safeguards
            .lock()
            .unwrap()
            .iter_mut()
            .find(|x| x.id == id)
    {
        x.evidence = Some(eid)
    }
    s.0.audit.write(serde_json::json!({"at":now(),"event":"evidence_metadata_added","safeguard_id":id,"encrypted_at_rest":false,"malware_scanned":false}).to_string());
    StatusCode::CREATED
}
fn router(s: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/api/summary", get(summary))
        .route("/safeguards/:id/toggle", post(toggle_safeguard))
        .route("/training/:id/toggle", post(toggle_training))
        .route("/safeguards/:id/evidence", post(add_evidence))
        .layer(middleware::from_fn_with_state(s.clone(), audit))
        .with_state(s)
}
#[tokio::main]
async fn main() {
    let raw = std::env::var("SYNTHETIC_DATA")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| EMBEDDED.into());
    let s = AppState::load(&raw, AuditSink::Stdout).expect("synthetic dataset required");
    let addr = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let l = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("compliance checklist preview listening on http://{addr}");
    axum::serve(l, router(s)).await.unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    fn state() -> AppState {
        AppState::load(EMBEDDED, AuditSink::Memory(Mutex::new(vec![]))).unwrap()
    }
    #[test]
    fn refuses_non_synthetic() {
        assert!(AppState::load(
            &EMBEDDED.replace("SYNTHETIC DATA", "SAMPLE DATA"),
            AuditSink::Stdout
        )
        .is_err())
    }
    #[tokio::test]
    async fn health_and_html() {
        let r = router(state())
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(r.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("text/html"))
    }
    #[tokio::test]
    async fn safeguard_and_evidence_work() {
        let s = state();
        let app = router(s.clone());
        let r = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/safeguards/sg-01/toggle")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        assert!(s.0.safeguards.lock().unwrap()[0].complete);
        let app = router(s.clone());
        let r = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .uri("/safeguards/sg-01/evidence")
                    .body(Body::from("label=access-review-notes"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        assert_eq!(s.0.evidence.lock().unwrap().len(), 1);
        assert!(!s.0.evidence.lock().unwrap()[0].encrypted_at_rest)
    }
}
