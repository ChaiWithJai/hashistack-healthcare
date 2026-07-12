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
            return Err("SYNTHETIC marker required".into());
        }
        Ok(Self(Arc::new(Inner {
            dataset: d.dataset,
            seed: d.items[0].clone(),
            audit: Mutex::new(vec![]),
        })))
    }
}
async fn audit(State(s): State<App>, r: Request, n: Next) -> Response {
    let o = n.run(r).await;
    let l=serde_json::json!({"event":"local_request","status":o.status().as_u16(),"source_retained":false}).to_string();
    println!("{l}");
    s.0.audit.lock().unwrap().push(l);
    o
}
async fn health(State(s): State<App>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","mode":"local-offline","dataset":s.0.dataset}))
}
fn page(seed: &str, out: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang="en"><body><nav>Local workspace</nav><main><h1>Local note extraction</h1><p><strong>SYNTHETIC DATA.</strong> Human confirmation required. This deterministic offline draft is not clinical advice and must not drive care.</p><form method=post action=/process><label for=source>Source note</label><textarea id=source name=source>{seed}</textarea><button>Extract draft</button></form><h2>Draft — Human confirmation required</h2><pre>{out}</pre></main></body></html>"#
    ))
}
async fn home(State(s): State<App>) -> Html<String> {
    page(&s.0.seed, "")
}
#[derive(Deserialize)]
struct Input {
    source: String,
}
fn extract(x: &str) -> String {
    x.split('.')
        .filter_map(|p| p.split_once(':'))
        .map(|(k, v)| format!("{}: {}", k.trim(), v.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}
async fn process(Form(x): Form<Input>) -> Html<String> {
    page(&x.source, &extract(&x.source))
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
    fn extracts_fields() {
        assert!(extract("Date: today. Action: return.").contains("Action: return"))
    }
    #[tokio::test]
    async fn boots() {
        assert_eq!(
            router(App::load(DATA).unwrap())
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            200
        )
    }
}
