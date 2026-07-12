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
const DATA: &str = include_str!("../../synthetic/requests.json");
#[derive(Deserialize)]
struct Dataset {
    dataset: String,
    notice: String,
    slots: Vec<String>,
    requests: Vec<Appointment>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Appointment {
    id: String,
    name: String,
    reason: String,
    preference: String,
    status: String,
    offered_slot: Option<String>,
}
struct Inner {
    dataset: String,
    slots: Vec<String>,
    requests: Mutex<Vec<Appointment>>,
    audit: Mutex<Vec<String>>,
}
#[derive(Clone)]
struct App(Arc<Inner>);
impl App {
    fn load(raw: &str) -> Result<Self, String> {
        let d: Dataset = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if !d.notice.contains("SYNTHETIC") {
            return Err("refusing non-SYNTHETIC dataset".into());
        }
        Ok(Self(Arc::new(Inner {
            dataset: d.dataset,
            slots: d.slots,
            requests: Mutex::new(d.requests),
            audit: Mutex::new(vec![]),
        })))
    }
}
fn urgent(s: &str) -> bool {
    let x = s.to_lowercase();
    [
        "chest pain",
        "can't breathe",
        "cannot breathe",
        "suicidal",
        "stroke",
        "severe bleeding",
        "emergency",
    ]
    .iter()
    .any(|w| x.contains(w))
}
async fn audit(State(s): State<App>, r: Request, n: Next) -> Response {
    let path = r.uri().path().to_string();
    let method = r.method().to_string();
    let out = n.run(r).await;
    let line=serde_json::json!({"event":"http_request","method":method,"path":path,"status":out.status().as_u16(),"durable":false}).to_string();
    println!("{line}");
    s.0.audit.lock().unwrap().push(line);
    out
}
async fn health(State(s): State<App>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","mode":"synthetic","dataset":s.0.dataset}))
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
async fn home(State(s): State<App>) -> Html<String> {
    let rows=s.0.requests.lock().unwrap().iter().map(|r|format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><form method=post action=/requests/{}/escalate><button>Send to manual review</button></form></td></tr>",r.id,esc(&r.name),esc(&r.reason),esc(&r.preference),r.status,r.offered_slot.as_deref().unwrap_or("none"),r.id)).collect::<String>();
    let opts =
        s.0.slots
            .iter()
            .map(|x| format!("<option>{}</option>", x))
            .collect::<String>();
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Appointment requests</title><style>body{{font:16px system-ui;max-width:1100px;margin:auto;padding:2rem}}table{{width:100%;border-collapse:collapse}}th,td{{padding:.6rem;border:1px solid #ccc;text-align:left}}.warn{{padding:1rem;background:#fff3cd}}label{{display:block;margin:.5rem 0}}:focus{{outline:3px solid #165d50}}</style></head><body><main><h1>Appointment requests</h1><p class=warn><strong>Synthetic preview.</strong> This administrative tool does not provide medical triage or medical advice. Urgent language is not ranked or diagnosed: it is held for immediate staff review with instructions to use emergency services when appropriate. Authentication, integrations, encryption, and durable audit storage remain unconfigured.</p><form method=post action=/requests><h2>New request</h2><label>Name <input name=name required></label><label>Visit reason <input name=reason required></label><label>Preference <input name=preference required></label><button>Submit for staff review</button></form><p>Available synthetic slots: <select aria-label="Available slots">{opts}</select></p><table><caption>Inbound request inbox and availability offers</caption><thead><tr><th>ID</th><th>Name</th><th>Reason</th><th>Preference</th><th>Status</th><th>Offer</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table></main></body></html>"#
    ))
}
#[derive(Deserialize)]
struct New {
    name: String,
    reason: String,
    preference: String,
}
async fn create(State(s): State<App>, Form(f): Form<New>) -> impl IntoResponse {
    if f.name.trim().is_empty() || f.reason.trim().is_empty() {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }
    let manual = urgent(&f.reason);
    let mut rs = s.0.requests.lock().unwrap();
    let id = format!("req-{}", 100 + rs.len() + 1);
    rs.push(Appointment {
        id,
        name: f.name,
        reason: f.reason,
        preference: f.preference,
        status: if manual { "manual-review" } else { "new" }.into(),
        offered_slot: None,
    });
    (StatusCode::SEE_OTHER, [(axum::http::header::LOCATION, "/")]).into_response()
}
#[derive(Deserialize)]
struct Offer {
    slot: String,
}
async fn offer(Path(id): Path<String>, State(s): State<App>, Form(f): Form<Offer>) -> StatusCode {
    if !s.0.slots.contains(&f.slot) {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    let mut rs = s.0.requests.lock().unwrap();
    let Some(r) = rs.iter_mut().find(|r| r.id == id) else {
        return StatusCode::NOT_FOUND;
    };
    if r.status == "manual-review" {
        return StatusCode::CONFLICT;
    }
    r.offered_slot = Some(f.slot);
    r.status = "offered".into();
    StatusCode::NO_CONTENT
}
async fn escalate(Path(id): Path<String>, State(s): State<App>) -> impl IntoResponse {
    let mut rs = s.0.requests.lock().unwrap();
    let Some(r) = rs.iter_mut().find(|r| r.id == id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    r.status = "manual-review".into();
    r.offered_slot = None;
    (StatusCode::SEE_OTHER, [(axum::http::header::LOCATION, "/")]).into_response()
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/requests", post(create))
        .route("/requests/:id/offer", post(offer))
        .route("/requests/:id/escalate", post(escalate))
        .layer(middleware::from_fn_with_state(s.clone(), audit))
        .with_state(s)
}
#[tokio::main]
async fn main() {
    let raw = std::env::var("SYNTHETIC_DATA")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| DATA.into());
    let s = App::load(&raw).expect("synthetic dataset required");
    let addr = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let l = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(l, router(s)).await.unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[test]
    fn rejects_real_data() {
        assert!(App::load(&DATA.replace("SYNTHETIC DATA", "REAL DATA")).is_err())
    }
    #[tokio::test]
    async fn accessible_page() {
        let r = router(App::load(DATA).unwrap())
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), 200)
    }
    #[tokio::test]
    async fn urgent_text_never_gets_slot() {
        let s = App::load(DATA).unwrap();
        let app = router(s.clone());
        let r = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/requests")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "name=Sam&reason=chest+pain+today&preference=soon",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 303);
        assert_eq!(
            s.0.requests.lock().unwrap().last().unwrap().status,
            "manual-review"
        );
        let app = router(s);
        let r = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/requests/req-103/offer")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("slot=2026-07-14+09%3A00"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 409)
    }
}
