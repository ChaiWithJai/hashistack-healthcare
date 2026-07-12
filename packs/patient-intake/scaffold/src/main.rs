use axum::{
    extract::{Form, Path, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const DATA: &str = include_str!("../../synthetic/intake-demo.json");
#[derive(Clone, Deserialize)]
struct Dataset {
    #[serde(rename = "dataset")]
    _dataset: String,
    notice: String,
    invites: Vec<Invite>,
}
#[derive(Clone, Deserialize)]
struct Invite {
    token: String,
    expires_at: u64,
    patient_id: String,
    name: String,
    visit: String,
}
#[derive(Clone, Deserialize)]
struct Intake {
    history: String,
    medications: String,
    allergies: String,
    concerns: String,
}
struct Inner {
    data: Dataset,
    completed: Mutex<HashMap<String, Intake>>,
    sessions: Mutex<HashMap<String, Instant>>,
    audit: Mutex<Vec<String>>,
    idle: Duration,
}
#[derive(Clone)]
struct App(Arc<Inner>);
impl App {
    fn load(x: &str, idle: Duration) -> Result<Self, String> {
        let data: Dataset = serde_json::from_str(x).map_err(|e| e.to_string())?;
        if !data.notice.contains("SYNTHETIC") {
            return Err("refusing to boot: dataset is not marked SYNTHETIC".into());
        }
        Ok(Self(Arc::new(Inner {
            data,
            completed: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            audit: Mutex::new(vec![]),
            idle,
        })))
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn cookie(h: &HeaderMap) -> Option<String> {
    h.get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|x| x.trim().strip_prefix("session=").map(str::to_owned))
}
async fn session(State(s): State<App>, r: Request, n: Next) -> Response {
    if r.uri().path() == "/health" {
        return n.run(r).await;
    }
    if let Some(id) = cookie(r.headers()) {
        let expired = {
            let mut ss = s.0.sessions.lock().unwrap();
            let expired = ss.get(&id).is_none_or(|t| t.elapsed() > s.0.idle);
            if expired {
                ss.remove(&id);
            } else {
                ss.insert(id, Instant::now());
            }
            expired
        };
        if expired {
            return (
                StatusCode::UNAUTHORIZED,
                Html(page(
                    "Session expired",
                    "The idle session ended. Reopen the synthetic invite.",
                )),
            )
                .into_response();
        }
        return n.run(r).await;
    }
    let id = format!("intake-{}", s.0.sessions.lock().unwrap().len() + 1);
    s.0.sessions
        .lock()
        .unwrap()
        .insert(id.clone(), Instant::now());
    let mut out = n.run(r).await;
    out.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!("session={id}; Path=/; HttpOnly; SameSite=Lax")).unwrap(),
    );
    out
}
async fn audit(State(s): State<App>, r: Request, n: Next) -> Response {
    let path = r.uri().path().to_owned();
    let method = r.method().to_string();
    let o = n.run(r).await;
    let line=serde_json::json!({"event":"route_access","path":path,"method":method,"status":o.status().as_u16(),"synthetic":true}).to_string();
    println!("{line}");
    s.0.audit.lock().unwrap().push(line);
    o
}
fn page(t: &str, b: &str) -> String {
    format!(
        r#"<!doctype html><html lang=en><meta name=viewport content="width=device-width"><title>{t}</title><style>body{{font:17px system-ui;max-width:850px;margin:3rem auto;padding:1rem;background:#f6f4ef;color:#20242d}}main{{background:white;padding:2rem;border-radius:14px}}label,textarea,button{{display:block;margin:.7rem 0}}textarea{{width:95%;min-height:70px;padding:.6rem}}button{{padding:.7rem}}</style><main><small>SYNTHETIC TRAINING APP</small><h1>{t}</h1>{b}<footer><small>This learning app is not monitored in real time. Authentication, encrypted storage, and durable audit retention are required before real use.</small></footer></main></html>"#
    )
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","synthetic_only":true}))
}
fn invite<'a>(s: &'a App, t: &str) -> Option<&'a Invite> {
    s.0.data.invites.iter().find(|i| i.token == t)
}
async fn form(State(s): State<App>, Path(token): Path<String>) -> Response {
    let Some(i) = invite(&s, &token) else {
        return (StatusCode::NOT_FOUND, "unknown invite").into_response();
    };
    if i.expires_at <= now() {
        return (
            StatusCode::GONE,
            Html(page(
                "Link expired",
                "This intake link expired. Ask the practice for a new link.",
            )),
        )
            .into_response();
    }
    Html(page("Pre-visit intake",&format!("<p><b>{}</b> · {}. Submit before the visit. For urgent symptoms, call emergency services; this form is not monitored in real time.</p><form method=post><label>Medical and surgical history<textarea name=history required></textarea></label><label>Medications<textarea name=medications required></textarea></label><label>Allergies and reactions<textarea name=allergies required></textarea></label><label>Visit concerns<textarea name=concerns required></textarea></label><button>Send structured intake</button></form>",i.name,i.visit))).into_response()
}
async fn submit(
    State(s): State<App>,
    Path(token): Path<String>,
    Form(x): Form<Intake>,
) -> Response {
    let Some(i) = invite(&s, &token) else {
        return (StatusCode::NOT_FOUND, "unknown invite").into_response();
    };
    if i.expires_at <= now() {
        return (StatusCode::GONE, "invite expired").into_response();
    }
    let id = i.patient_id.clone();
    s.0.completed.lock().unwrap().insert(id.clone(), x);
    Html(page("Intake received",&format!("<p>Structured summary queued for chart review under synthetic patient <b>{id}</b>.</p><p>Staff must reconcile medications and allergies; submission does not create clinical orders.</p>"))).into_response()
}
async fn summary(State(s): State<App>, Path(id): Path<String>) -> Response {
    let c = s.0.completed.lock().unwrap();
    let Some(x) = c.get(&id) else {
        return (StatusCode::NOT_FOUND, "no completed intake").into_response();
    };
    Html(page("Chart summary",&format!("<dl><dt>History</dt><dd>{}</dd><dt>Medications</dt><dd>{}</dd><dt>Allergies</dt><dd>{}</dd><dt>Concerns</dt><dd>{}</dd></dl><p><b>Reconciliation required:</b> patient-reported, not yet clinician verified.</p>",x.history,x.medications,x.allergies,x.concerns))).into_response()
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/intake/:token", get(form).post(submit))
        .route("/summary/:id", get(summary))
        .layer(middleware::from_fn_with_state(s.clone(), session))
        .layer(middleware::from_fn_with_state(s.clone(), audit))
        .with_state(s)
}
#[tokio::main]
async fn main() {
    let raw = std::env::var("SYNTHETIC_DATA")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| DATA.into());
    let s = App::load(&raw, Duration::from_secs(900)).expect("synthetic seed required");
    let bind = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let l = tokio::net::TcpListener::bind(bind).await.unwrap();
    axum::serve(l, router(s)).await.unwrap()
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
        assert!(App::load(
            r#"{"dataset":"x","notice":"real","invites":[]}"#,
            Duration::ZERO
        )
        .is_err())
    }
    #[tokio::test]
    async fn expiry_and_summary() {
        let s = App::load(DATA, Duration::from_secs(30)).unwrap();
        let a = router(s.clone());
        assert_eq!(
            a.clone()
                .oneshot(
                    Request::get("/intake/expired-demo")
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            StatusCode::GONE
        );
        let r=a.clone().oneshot(Request::post("/intake/demo-alex").header("content-type","application/x-www-form-urlencoded").body(Body::from("history=none&medications=lisinopril&allergies=penicillin&concerns=headache")).unwrap()).await.unwrap();
        assert_eq!(r.status(), 200);
        let r = a
            .oneshot(Request::get("/summary/in-001").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let b =
            String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
        assert!(b.contains("lisinopril") && b.contains("Reconciliation required"));
        assert!(!s.0.audit.lock().unwrap().is_empty())
    }
}
