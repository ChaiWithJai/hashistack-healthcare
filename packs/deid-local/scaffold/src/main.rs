use axum::{
    extract::{Form, Request, State},
    middleware::{self, Next},
    response::{Html, Response},
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
    audit: Mutex<Vec<String>>,
}
#[derive(Clone)]
struct App(Arc<Inner>);
impl App {
    fn load(x: &str) -> Result<Self, String> {
        let d: Dataset = serde_json::from_str(x).map_err(|e| e.to_string())?;
        if !d.notice.contains("SYNTHETIC") {
            return Err("refusing dataset not marked SYNTHETIC".into());
        }
        Ok(Self(Arc::new(Inner {
            dataset: d.dataset,
            seed: d.items.into_iter().next().unwrap_or_default(),
            audit: Mutex::new(vec![]),
        })))
    }
}
async fn audit(State(s): State<App>, r: Request, n: Next) -> Response {
    let path = r.uri().path().to_string();
    let o = n.run(r).await;
    let l=serde_json::json!({"event":"local_request","path":path,"status":o.status().as_u16(),"source_retained":false}).to_string();
    println!("{l}");
    s.0.audit.lock().unwrap().push(l);
    o
}
async fn health(State(s): State<App>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","mode":"local-offline","dataset":s.0.dataset}))
}
fn page(seed: &str, out: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Local de-identification</title></head><body><nav>Local workspace</nav><main><h1>Local de-identification</h1><p><strong>SYNTHETIC DATA.</strong> Offline, rule-based preview; review required. It can miss identifiers and is not a compliance determination.</p><form method="post" action="/process"><label for="source">Source text</label><textarea id="source" name="source">{seed}</textarea><button>Create review draft</button></form><h2>Review draft</h2><pre>{out}</pre></main></body></html>"#
    ))
}
async fn home(State(s): State<App>) -> Html<String> {
    page(&s.0.seed, "")
}
#[derive(Deserialize)]
struct Input {
    source: String,
}
fn redact(x: &str) -> String {
    let mut o = x.to_string();
    for marker in ["Patient:", "DOB:", "Phone:"] {
        if let Some(i) = o.find(marker) {
            let start = i + marker.len();
            let end = o[start..].find(';').map(|n| start + n).unwrap_or(o.len());
            o.replace_range(start..end, " [REDACTED]")
        }
    }
    o.replace("Patient: [REDACTED]", "Patient: [NAME]")
}
async fn process(Form(x): Form<Input>) -> Html<String> {
    let o = redact(&x.source);
    page(&x.source, &o)
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/process", post(process))
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
        assert!(App::load(&DATA.replace("SYNTHETIC DATA", "REAL DATA")).is_err())
    }
    #[test]
    fn removes_identifiers() {
        let x = redact("Patient: Maya; DOB: 2000; Phone: 555;");
        assert!(!x.contains("Maya"));
        assert!(x.contains("[NAME]"))
    }
    #[tokio::test]
    async fn boots() {
        assert_eq!(
            router(App::load(DATA).unwrap())
                .oneshot(
                    Request::builder()
                        .uri("/health")
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            200
        )
    }
}
