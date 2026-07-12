use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
const DATA: &str = include_str!("../../synthetic/demo.json");
#[derive(Deserialize)]
struct Dataset {
    dataset: String,
    notice: String,
    items: Vec<String>,
}
struct Inner {
    dataset: String,
    seed: String,
    pending: Mutex<Option<String>>,
}
#[derive(Clone)]
struct App(Arc<Inner>);
impl App {
    fn load(x: &str) -> Result<Self, String> {
        let d: Dataset = serde_json::from_str(x).map_err(|e| e.to_string())?;
        if !d.notice.contains("SYNTHETIC") {
            return Err("SYNTHETIC marker required".into());
        }
        Ok(Self(Arc::new(Inner {
            dataset: d.dataset,
            seed: d.items[0].clone(),
            pending: Mutex::new(None),
        })))
    }
}
async fn health(State(s): State<App>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","mode":"local-first","dataset":s.0.dataset}))
}
fn redact(x: &str) -> String {
    let mut o = x.to_string();
    for m in ["Patient:", "Phone:"] {
        if let Some(i) = o.find(m) {
            let a = i + m.len();
            let b = o[a..].find(';').map(|n| a + n).unwrap_or(o.len());
            o.replace_range(a..b, " [REDACTED]")
        }
    }
    o
}
fn page(seed: &str, out: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang="en"><body><nav>Local-first pipeline</nav><main><h1>Hybrid disclosure pipeline</h1><p><strong>SYNTHETIC DATA.</strong> Local redaction is a fallible rule-based preview. Nothing was transmitted. Any release requires explicit approval and an allowlisted destination.</p><form method=post action=/prepare><label for=source>Sensitive source text</label><textarea id=source name=source>{seed}</textarea><button>Prepare local preview</button></form><h2>Disclosure preview</h2><pre>{out}</pre><form method=post action=/approve><label><input type=checkbox name=approved value=true> I reviewed the disclosure</label><button>Approve synthetic release</button></form></main></body></html>"#
    ))
}
async fn home(State(s): State<App>) -> Html<String> {
    page(&s.0.seed, "")
}
#[derive(Deserialize)]
struct Input {
    source: String,
}
async fn prepare(State(s): State<App>, Form(i): Form<Input>) -> Html<String> {
    let out = redact(&i.source);
    *s.0.pending.lock().unwrap() = Some(out.clone());
    page(&i.source, &out)
}
#[derive(Deserialize)]
struct Approval {
    approved: Option<bool>,
}
async fn approve(State(s): State<App>, Form(a): Form<Approval>) -> impl IntoResponse {
    if a.approved != Some(true) || s.0.pending.lock().unwrap().is_none() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"released":false})),
        );
    }
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"released":true,"mode":"synthetic-simulated","network_call":false}),
        ),
    )
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/prepare", post(prepare))
        .route("/approve", post(approve))
        .with_state(s)
}
#[tokio::main]
async fn main() {
    let raw = std::env::var("SYNTHETIC_DATA")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| DATA.into());
    let s = App::load(&raw).unwrap();
    let a = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let l = tokio::net::TcpListener::bind(a).await.unwrap();
    axum::serve(l, router(s)).await.unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[test]
    fn refuses_real() {
        assert!(App::load(&DATA.replace("SYNTHETIC DATA", "REAL")).is_err())
    }
    #[test]
    fn redacts() {
        assert!(!redact("Patient: Maya; Phone: 555;").contains("Maya"))
    }
    #[tokio::test]
    async fn approval_without_preview_fails() {
        let r = router(App::load(DATA).unwrap())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/approve")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("approved=true"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 409)
    }
}
