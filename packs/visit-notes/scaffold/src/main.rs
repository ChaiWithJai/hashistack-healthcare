use axum::{
    extract::{DefaultBodyLimit, Form, State},
    http::header,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
#[allow(dead_code)]
mod local_media;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
const DATA: &str = include_str!("../../synthetic/demo.json");
#[derive(Clone, Deserialize)]
struct Seed {
    notice: String,
    events: Vec<Event>,
}
#[derive(Clone, Deserialize, serde::Serialize)]
struct Event {
    source: String,
    text: String,
    flag: bool,
}
#[derive(Clone)]
struct App(Arc<Inner>);
struct Inner {
    seed: Seed,
    drafts: Mutex<Vec<String>>,
}
impl App {
    fn load(x: &str) -> Result<Self, String> {
        let seed: Seed = serde_json::from_str(x).map_err(|e| e.to_string())?;
        if !seed.notice.contains("SYNTHETIC") {
            return Err("SYNTHETIC fixture required".into());
        }
        Ok(Self(Arc::new(Inner {
            seed,
            drafts: Mutex::new(vec![]),
        })))
    }
}
fn page(b: &str) -> String {
    format!(
        r#"<!doctype html><html lang=en><meta name=viewport content="width=device-width"><title>Visit notes</title><style>body{{font:16px system-ui;max-width:850px;margin:2rem auto}}nav,main{{padding:1rem}}label,textarea,button{{display:block;margin:.6rem;padding:.5rem}}textarea{{width:90%}}</style><nav aria-label="Visit note">Visit-notes learning stream</nav><main><small>SYNTHETIC DATA · no real audio service connected</small><h1>Transcript to unsigned draft</h1>{b}<p>Human clinician review and signature are required. This app does not create a chart note or provide clinical decision support.</p></main></html>"#
    )
}
async fn home(State(s): State<App>) -> Html<String> {
    let transcript =
        s.0.seed
            .events
            .iter()
            .map(|e| format!("<li><b>{}</b>: {}</li>", e.source, e.text))
            .collect::<String>();
    Html(page(&format!("<ol>{transcript}</ol><form method=post action=/draft><label>Unsigned draft<textarea name=text required></textarea></label><button>Save draft for review</button></form>")))
}
#[derive(Deserialize)]
struct Draft {
    text: String,
}
async fn draft(State(s): State<App>, Form(d): Form<Draft>) -> Html<String> {
    s.0.drafts.lock().unwrap().push(d.text);
    Html(page(
        "<p>Unsigned draft saved for clinician review. It was not placed in a chart.</p>",
    ))
}
async fn stream(State(s): State<App>) -> Response {
    let x =
        s.0.seed
            .events
            .iter()
            .map(|e| format!("data: {}\n\n", serde_json::to_string(e).unwrap()))
            .collect::<String>();
    ([(header::CONTENT_TYPE, "text/event-stream")], x).into_response()
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","synthetic_only":true}))
}
fn app(s: App) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/stream", get(stream))
        .route("/draft", post(draft))
        .route(
            "/api/local-media/capabilities",
            get(local_media::capabilities_audio),
        )
        .route("/api/local-media/audio", post(local_media::audio))
        .route("/health", get(health))
        .layer(DefaultBodyLimit::max(25 * 1024 * 1024))
        .with_state(s)
}
#[tokio::main]
async fn main() {
    let x = std::env::var("SYNTHETIC_DATA")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| DATA.into());
    let s = App::load(&x).expect("synthetic fixture");
    let b = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let l = tokio::net::TcpListener::bind(b).await.unwrap();
    axum::serve(l, app(s)).await.unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::ServiceExt;
    #[test]
    fn rejects_real() {
        assert!(App::load(r#"{"notice":"real","events":[]}"#).is_err())
    }
    #[tokio::test]
    async fn draft_is_unsigned() {
        let s = App::load(DATA).unwrap();
        let a = app(s.clone());
        let r = a
            .oneshot(
                Request::post("/draft")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("text=synthetic+draft"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec())
                .unwrap()
                .contains("Unsigned draft")
        );
        assert_eq!(s.0.drafts.lock().unwrap().len(), 1);
    }
}
