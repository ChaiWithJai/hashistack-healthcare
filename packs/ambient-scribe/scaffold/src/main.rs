use axum::{
    extract::{Form, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
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
    draft: Mutex<String>,
}
impl App {
    fn load(x: &str) -> Result<Self, String> {
        let seed: Seed = serde_json::from_str(x).map_err(|e| e.to_string())?;
        if !seed.notice.contains("SYNTHETIC") {
            return Err("SYNTHETIC fixture required".into());
        }
        Ok(Self(Arc::new(Inner {
            seed,
            draft: Mutex::new(String::new()),
        })))
    }
}
fn page(b: &str) -> String {
    format!(
        r#"<!doctype html><html lang=en><meta name=viewport content="width=device-width"><title>Ambient scribe</title><style>body{{font:16px system-ui;max-width:850px;margin:2rem auto}}nav,main{{padding:1rem}}label,textarea,button{{display:block;margin:.6rem;padding:.5rem}}textarea{{width:90%}}</style><nav aria-label="Scribe">Ambient-scribe learning stream</nav><main><small>SYNTHETIC DATA · consent simulated · no microphone or vendor connected</small><h1>Consent-bound ambient draft</h1>{b}<p>Unsigned draft only. Clinician correction and signature are required; this is not clinical decision support.</p></main></html>"#
    )
}
async fn home(State(s): State<App>) -> Html<String> {
    let t =
        s.0.seed
            .events
            .iter()
            .map(|e| format!("<li><b>{}</b>: {}</li>", e.source, e.text))
            .collect::<String>();
    Html(page(&format!("<p><b>Consent:</b> synthetic demo consent recorded.</p><ul>{t}</ul><form method=post action=/draft><label>SOAP draft<textarea name=text required></textarea></label><button>Save unsigned draft</button></form>")))
}
#[derive(Deserialize)]
struct Draft {
    text: String,
}
async fn draft(State(s): State<App>, Form(d): Form<Draft>) -> Html<String> {
    *s.0.draft.lock().unwrap() = d.text;
    Html(page(
        "<p>Unsigned SOAP draft saved in memory for clinician correction.</p>",
    ))
}
async fn sign() -> Response {
    (StatusCode::FORBIDDEN,Html(page("<p>This scaffold refuses autonomous signature. A clinician must review in the system of record.</p>"))).into_response()
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
        .route("/sign", post(sign))
        .route("/health", get(health))
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
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[test]
    fn rejects_real() {
        assert!(App::load(r#"{"notice":"real","events":[]}"#).is_err())
    }
    #[tokio::test]
    async fn saves_but_refuses_sign() {
        let s = App::load(DATA).unwrap();
        let a = app(s.clone());
        assert_eq!(
            a.clone()
                .oneshot(
                    Request::post("/draft")
                        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                        .body(Body::from("text=subjective+synthetic"))
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(&*s.0.draft.lock().unwrap(), "subjective synthetic");
        assert_eq!(
            a.oneshot(Request::post("/sign").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            403
        )
    }
}
