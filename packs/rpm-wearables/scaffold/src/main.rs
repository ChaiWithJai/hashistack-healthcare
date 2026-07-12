use axum::{
    extract::{Form, Query, State},
    http::header,
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
    review: Mutex<Vec<String>>,
    audit: Mutex<Vec<String>>,
}
impl App {
    fn load(x: &str) -> Result<Self, String> {
        let seed: Seed = serde_json::from_str(x).map_err(|e| e.to_string())?;
        if !seed.notice.contains("SYNTHETIC") {
            return Err("SYNTHETIC fixture required".into());
        }
        Ok(Self(Arc::new(Inner {
            seed,
            review: Mutex::new(vec![]),
            audit: Mutex::new(vec![]),
        })))
    }
}
fn page(b: &str) -> String {
    format!(
        r#"<!doctype html><html lang=en><meta name=viewport content="width=device-width"><title>RPM wearable stream</title><style>body{{font:16px system-ui;max-width:900px;margin:2rem auto;padding:1rem}}nav,main{{padding:1rem}}table{{width:100%;border-collapse:collapse}}th,td{{padding:.7rem;border-bottom:1px solid #ddd;text-align:left}}label,select,button{{padding:.5rem;margin:.4rem}}</style><nav aria-label="Monitoring">RPM learning stream</nav><main><small>SYNTHETIC DATA · demo credential boundary not production authentication</small><h1>Wearable observation stream</h1>{b}<p>Threshold flags require human review. This is not an emergency service or clinical decision support.</p></main></html>"#
    )
}
#[derive(Default, Deserialize)]
struct Filter {
    source: Option<String>,
}
async fn home(State(s): State<App>, Query(f): Query<Filter>) -> Html<String> {
    let rows: Vec<_> =
        s.0.seed
            .events
            .iter()
            .filter(|e| {
                f.source
                    .as_deref()
                    .filter(|x| *x != "all")
                    .is_none_or(|x| e.source == x)
            })
            .collect();
    let mut b=r#"<form><label>Device source<select name=source><option>all</option><option>watch-01</option><option>cuff-01</option></select></label><button>Filter stream</button></form><table aria-label="Synthetic wearable events"><tr><th>Source</th><th>Observation</th><th>Review</th></tr>"#.to_string();
    for e in rows {
        b.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            e.source,
            e.text,
            if e.flag {
                "human review queued"
            } else {
                "routine"
            }
        ))
    }
    b.push_str("</table><form method=post action=/review><label>Review note<input name=note required></label><button>Record human review</button></form>");
    Html(page(&b))
}
#[derive(Deserialize)]
struct Note {
    note: String,
}
async fn review(State(s): State<App>, Form(n): Form<Note>) -> Html<String> {
    s.0.review.lock().unwrap().push(n.note);
    s.0.audit
        .lock()
        .unwrap()
        .push("human_review_recorded".into());
    Html(page(
        "<p>Human review recorded in the synthetic in-memory queue.</p>",
    ))
}
async fn stream(State(s): State<App>) -> Response {
    let data =
        s.0.seed
            .events
            .iter()
            .map(|e| format!("data: {}\n\n", serde_json::to_string(e).unwrap()))
            .collect::<String>();
    ([(header::CONTENT_TYPE, "text/event-stream")], data).into_response()
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","synthetic_only":true}))
}
fn app(s: App) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/stream", get(stream))
        .route("/review", post(review))
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
    async fn filter_and_stream() {
        let s = App::load(DATA).unwrap();
        let a = app(s);
        let r = a
            .clone()
            .oneshot(
                Request::get("/?source=cuff-01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let b =
            String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
        assert!(b.contains("184/112") && !b.contains("72 bpm"));
        let r = a
            .oneshot(Request::get("/stream").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.headers()[header::CONTENT_TYPE], "text/event-stream")
    }
}
