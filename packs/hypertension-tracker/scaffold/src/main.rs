use axum::{
    extract::{Form, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
const DATA: &str = include_str!("../../synthetic/htn-demo.json");
#[derive(Clone, Deserialize)]
struct Dataset {
    #[serde(rename = "dataset")]
    _dataset: String,
    notice: String,
    patients: Vec<Patient>,
}
#[derive(Clone, Deserialize)]
struct Patient {
    id: String,
    name: String,
    target_systolic: u16,
    target_diastolic: u16,
    readings: Vec<Reading>,
}
#[derive(Clone, Deserialize)]
struct Reading {
    systolic: u16,
    diastolic: u16,
}
#[derive(Deserialize)]
struct Entry {
    patient_id: String,
    systolic: u16,
    diastolic: u16,
}
struct Inner {
    data: Dataset,
    added: Mutex<Vec<(String, Reading)>>,
    alerts: Mutex<Vec<String>>,
    sessions: Mutex<HashMap<String, Instant>>,
    audit: Mutex<Vec<String>>,
    idle: Duration,
}
#[derive(Clone)]
struct App(Arc<Inner>);
impl App {
    fn load(s: &str, idle: Duration) -> Result<Self, String> {
        let data: Dataset = serde_json::from_str(s).map_err(|e| e.to_string())?;
        if !data.notice.contains("SYNTHETIC") {
            return Err("refusing to boot: dataset is not marked SYNTHETIC".into());
        }
        Ok(Self(Arc::new(Inner {
            data,
            added: Mutex::new(vec![]),
            alerts: Mutex::new(vec![]),
            sessions: Mutex::new(HashMap::new()),
            audit: Mutex::new(vec![]),
            idle,
        })))
    }
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
                    "Your idle session ended. Reload to start a new synthetic session.",
                )),
            )
                .into_response();
        }
        return n.run(r).await;
    }
    let id = format!("demo-{}", s.0.sessions.lock().unwrap().len() + 1);
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
    let method = r.method().to_string();
    let path = r.uri().path().to_string();
    let out = n.run(r).await;
    let line=serde_json::json!({"event":"route_access","method":method,"path":path,"status":out.status().as_u16(),"dataset":"SYNTHETIC"}).to_string();
    println!("{line}");
    s.0.audit.lock().unwrap().push(line);
    out
}
fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><meta name="viewport" content="width=device-width"><title>{title}</title><style>body{{font:17px system-ui;max-width:850px;margin:3rem auto;padding:1rem;background:#f5f7fb;color:#172033}}main{{background:white;padding:2rem;border-radius:16px}}label,input,button{{display:block;margin:.65rem 0}}input,button{{padding:.7rem}}.alert{{color:#9b1c1c;font-weight:700}}</style><main><small>SYNTHETIC TRAINING APP · not medical advice</small><h1>{title}</h1>{body}</main></html>"#
    )
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","synthetic_only":true}))
}
async fn home(State(s): State<App>) -> Html<String> {
    let rows =
        s.0.data
            .patients
            .iter()
            .map(|p| {
                let r = p.readings.last().unwrap();
                format!(
                    "<li><b>{}</b>: {}/{} · target under {}/{}</li>",
                    p.name, r.systolic, r.diastolic, p.target_systolic, p.target_diastolic
                )
            })
            .collect::<String>();
    let alerts =
        s.0.alerts
            .lock()
            .unwrap()
            .iter()
            .map(|alert| format!("<li data-testid=alert-row>{alert}</li>"))
            .collect::<String>();
    Html(page("Home blood-pressure log",&format!("<p>Log a resting reading. Values ≥180 systolic or ≥120 diastolic are labeled urgent; 140/90 or above routes a clinician review. This app does not replace emergency care.</p><h2>Trends and target bands</h2><ul>{rows}</ul><h2>Clinician inbox</h2><ul>{alerts}</ul><form method=post action=/readings><label>Patient ID<input name=patient_id required></label><label>Systolic<input type=number name=systolic min=60 max=260 required></label><label>Diastolic<input type=number name=diastolic min=30 max=180 required></label><button>Save reading</button></form>")))
}
async fn add(State(s): State<App>, Form(e): Form<Entry>) -> Response {
    if !s.0.data.patients.iter().any(|p| p.id == e.patient_id) {
        return (StatusCode::BAD_REQUEST, "unknown synthetic patient").into_response();
    }
    let level = if e.systolic >= 180 || e.diastolic >= 120 {
        "URGENT: seek immediate clinical help; clinician inbox alerted"
    } else if e.systolic >= 140 || e.diastolic >= 90 {
        "OUT OF RANGE: clinician review requested"
    } else {
        "in target review range"
    };
    if level != "in target review range" {
        s.0.alerts.lock().unwrap().push(format!(
            "{} {}/{}: {level}",
            e.patient_id, e.systolic, e.diastolic
        ))
    }
    s.0.added.lock().unwrap().push((
        e.patient_id,
        Reading {
            systolic: e.systolic,
            diastolic: e.diastolic,
        },
    ));
    Html(page(
        "Reading recorded",
        &format!(
            "<p class=alert>{level}</p><p>{}/{}</p><a href=/ >Back</a>",
            e.systolic, e.diastolic
        ),
    ))
    .into_response()
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/readings", post(add))
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
    fn refuses_real() {
        assert!(App::load(
            r#"{"dataset":"x","notice":"real","patients":[]}"#,
            Duration::ZERO
        )
        .is_err())
    }
    #[tokio::test]
    async fn health_and_escalation() {
        let s = App::load(DATA, Duration::from_secs(60)).unwrap();
        let app = router(s.clone());
        assert_eq!(
            app.clone()
                .oneshot(Request::get("/health").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            200
        );
        let r = app
            .oneshot(
                Request::post("/readings")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("patient_id=htn-001&systolic=181&diastolic=80"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(
            String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec())
                .unwrap()
                .contains("URGENT")
        );
        assert_eq!(s.0.alerts.lock().unwrap().len(), 1);
        assert!(!s.0.audit.lock().unwrap().is_empty())
    }
}
