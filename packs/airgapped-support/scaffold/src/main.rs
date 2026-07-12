use axum::{
    extract::{Form, State},
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
const DATA: &str = include_str!("../../synthetic/demo.json");
#[derive(Deserialize)]
struct Dataset {
    dataset: String,
    notice: String,
    items: Vec<String>,
}
#[derive(Clone)]
struct App(Arc<Dataset>);
impl App {
    fn load(x: &str) -> Result<Self, String> {
        let d: Dataset = serde_json::from_str(x).map_err(|e| e.to_string())?;
        if !d.notice.contains("SYNTHETIC") {
            return Err("SYNTHETIC marker required".into());
        }
        Ok(Self(Arc::new(d)))
    }
}
async fn health(State(s): State<App>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","network_required":false,"dataset":s.0.dataset}))
}
fn page(result: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang="en"><body><nav>Offline library</nav><main><h1>Offline support</h1><p><strong>SYNTHETIC DATA — offline preview.</strong> This is not emergency support. The bundled library may be stale; unresolved issues require human escalation.</p><form method=post action=/search><label for=q>Search query</label><input id=q name=q><button>Search local library</button></form><h2>Local results</h2><p>{result}</p><form method=post action=/ticket><label for=issue>Unresolved issue</label><input id=issue name=issue><button>Queue offline ticket</button></form></main></body></html>"#
    ))
}
async fn home() -> Html<String> {
    page("")
}
#[derive(Deserialize)]
struct Query {
    q: String,
}
async fn search(State(s): State<App>, Form(q): Form<Query>) -> Html<String> {
    let needle = q.q.to_lowercase();
    let found =
        s.0.items
            .iter()
            .find(|x| x.to_lowercase().contains(&needle))
            .map(String::as_str)
            .unwrap_or("No local match — queue for human support.");
    page(found)
}
#[derive(Deserialize)]
struct Ticket {
    issue: String,
}
async fn ticket(Form(t): Form<Ticket>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"status":"queued-locally","issue_length":t.issue.len(),"transmitted":false}),
    )
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/search", post(search))
        .route("/ticket", post(ticket))
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
    #[tokio::test]
    async fn search_is_local() {
        let r = router(App::load(DATA).unwrap())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/search")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("q=printer"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200)
    }
    #[tokio::test]
    async fn health() {
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
