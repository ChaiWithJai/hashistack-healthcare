use axum::{
    extract::{Form, Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
const DATA: &str = include_str!("../../synthetic/rides.json");
#[derive(Deserialize)]
struct Dataset {
    dataset: String,
    notice: String,
    rides: Vec<Ride>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Ride {
    id: String,
    rider: String,
    pickup: String,
    appointment: String,
    provider: String,
    status: String,
    escalated: bool,
}
struct Inner {
    dataset: String,
    rides: Mutex<Vec<Ride>>,
    audit: Mutex<Vec<String>>,
}
#[derive(Clone)]
struct App(Arc<Inner>);
impl App {
    fn load(s: &str) -> Result<Self, String> {
        let d: Dataset = serde_json::from_str(s).map_err(|e| e.to_string())?;
        if !d.notice.contains("SYNTHETIC") {
            return Err("refusing non-SYNTHETIC dataset".into());
        }
        Ok(Self(Arc::new(Inner {
            dataset: d.dataset,
            rides: Mutex::new(d.rides),
            audit: Mutex::new(vec![]),
        })))
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
async fn audit(State(s): State<App>, r: Request, n: Next) -> Response {
    let method = r.method().clone();
    let path = r.uri().path().to_string();
    let out = n.run(r).await;
    let line=serde_json::json!({"at":now(),"method":method.to_string(),"path":path,"status":out.status().as_u16(),"durable":false}).to_string();
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
    let rows=s.0.rides.lock().unwrap().iter().map(|r|format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><form method=post action=/rides/{}/escalate><button>Escalate</button></form></td></tr>",r.id,esc(&r.rider),esc(&r.pickup),r.appointment,esc(&r.provider),if r.escalated{"Escalated"}else{&r.status},r.id)).collect::<String>();
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Ride coordination</title><style>body{{font:16px system-ui;max-width:1100px;margin:auto;padding:2rem}}table{{width:100%;border-collapse:collapse}}th,td{{padding:.7rem;border:1px solid #ccc;text-align:left}}.warn{{padding:1rem;background:#fff3cd}}button{{padding:.5rem}}:focus{{outline:3px solid #1769aa}}</style></head><body><main><h1>Ride coordination</h1><p class=warn><strong>Synthetic preview; not an emergency service.</strong> Call emergency services for emergencies. Human follow-up required for late, missed, or unassigned rides. Provider integrations, authentication, encryption, and durable audit storage are not configured.</p><table><caption>Upcoming synthetic transportation</caption><thead><tr><th>Ride</th><th>Rider</th><th>Pickup</th><th>Appointment</th><th>Provider</th><th>Status</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table></main></body></html>"#
    ))
}
#[derive(Deserialize)]
struct Update {
    status: String,
}
async fn update(Path(id): Path<String>, State(s): State<App>, Form(f): Form<Update>) -> StatusCode {
    let allowed = [
        "scheduled",
        "driver-en-route",
        "arrived",
        "completed",
        "driver-late",
        "missed",
        "unassigned",
    ];
    if !allowed.contains(&f.status.as_str()) {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    let mut rs = s.0.rides.lock().unwrap();
    let Some(r) = rs.iter_mut().find(|r| r.id == id) else {
        return StatusCode::NOT_FOUND;
    };
    r.status = f.status;
    r.escalated = matches!(r.status.as_str(), "driver-late" | "missed" | "unassigned");
    StatusCode::NO_CONTENT
}
async fn escalate(Path(id): Path<String>, State(s): State<App>) -> impl IntoResponse {
    let mut rs = s.0.rides.lock().unwrap();
    let Some(r) = rs.iter_mut().find(|r| r.id == id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    r.escalated = true;
    (StatusCode::SEE_OTHER, [(axum::http::header::LOCATION, "/")]).into_response()
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/rides/:id/status", post(update))
        .route("/rides/:id/escalate", post(escalate))
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
    async fn html_and_health() {
        let r = router(App::load(DATA).unwrap())
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), 200)
    }
    #[tokio::test]
    async fn late_status_escalates() {
        let s = App::load(DATA).unwrap();
        let r = router(s.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rides/ride-101/status")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("status=missed"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 204);
        assert!(s.0.rides.lock().unwrap()[0].escalated)
    }
}
