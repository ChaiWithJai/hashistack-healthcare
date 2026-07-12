use axum::{
    extract::{Extension, Form, Request, State},
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
const DATA: &str = include_str!("../../synthetic/portal-demo.json");
#[derive(Clone, Deserialize)]
struct Dataset {
    notice: String,
    patients: Vec<Patient>,
}
#[derive(Clone, Deserialize)]
struct Patient {
    id: String,
    name: String,
    summary: String,
    appointment: String,
}
#[derive(Clone, PartialEq)]
enum Role {
    Patient,
    Clinician,
}
#[derive(Clone)]
struct Auth {
    role: Role,
    patient: Option<String>,
    actor: String,
}
struct Session {
    auth: Auth,
    last: Instant,
}
struct Inner {
    data: Dataset,
    sessions: Mutex<HashMap<String, Session>>,
    messages: Mutex<Vec<(String, String)>>,
    audit: Mutex<Vec<String>>,
    idle: Duration,
}
#[derive(Clone)]
struct App(Arc<Inner>);
impl App {
    fn load(x: &str) -> Result<Self, String> {
        let data: Dataset = serde_json::from_str(x).map_err(|e| e.to_string())?;
        if !data.notice.contains("SYNTHETIC") {
            return Err("refusing non-SYNTHETIC data".into());
        }
        Ok(Self(Arc::new(Inner {
            data,
            sessions: Mutex::new(HashMap::new()),
            messages: Mutex::new(vec![]),
            audit: Mutex::new(vec![]),
            idle: Duration::from_secs(900),
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
async fn auth(State(s): State<App>, mut r: Request, n: Next) -> Response {
    if matches!(r.uri().path(), "/health" | "/login") {
        return n.run(r).await;
    }
    let Some(id) = cookie(r.headers()) else {
        return unauthorized();
    };
    let a = {
        let mut ss = s.0.sessions.lock().unwrap();
        match ss.get_mut(&id) {
            Some(x) if x.last.elapsed() <= s.0.idle => {
                x.last = Instant::now();
                Some(x.auth.clone())
            }
            _ => {
                ss.remove(&id);
                None
            }
        }
    };
    let Some(a) = a else { return unauthorized() };
    r.extensions_mut().insert(a);
    n.run(r).await
}
async fn audit(State(s): State<App>, r: Request, n: Next) -> Response {
    let path = r.uri().path().to_string();
    let o = n.run(r).await;
    if path != "/health" {
        let l=serde_json::json!({"event":"portal_access","path":path,"status":o.status().as_u16(),"synthetic":true}).to_string();
        println!("{l}");
        s.0.audit.lock().unwrap().push(l)
    }
    o
}
fn page(t: &str, b: &str) -> String {
    format!(
        r#"<!doctype html><html lang=en><meta name=viewport content="width=device-width"><title>{t}</title><style>body{{font:16px system-ui;max-width:850px;margin:2rem auto;padding:1rem;background:#eef5f3}}main,nav{{background:white;padding:1.4rem;margin:.7rem;border-radius:14px}}label,input,textarea,button{{display:block;margin:.6rem 0}}input,textarea{{padding:.65rem;width:90%}}</style><nav aria-label="Portal"><b>Patient portal · learning app</b></nav><main><small>SYNTHETIC DATA · demo only · not monitored for emergencies</small><h1>{t}</h1>{b}</main></html>"#
    )
}
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Html(page(
            "Sign in required",
            "<a href=/login>Use demo sign in</a>",
        )),
    )
        .into_response()
}
#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}
async fn login_page() -> Html<String> {
    Html(page(
        "Demo sign in",
        r#"<p>Demo-only credentials, never use real secrets: <code>patient / learn-patient</code>, <code>clinician / learn-clinician</code>.</p><form method=post><label>Username<input name=username></label><label>Password<input type=password name=password></label><button>Sign in</button></form>"#,
    ))
}
async fn login(State(s): State<App>, Form(x): Form<Login>) -> Response {
    let a = match (x.username.as_str(), x.password.as_str()) {
        ("patient", "learn-patient") => Auth {
            role: Role::Patient,
            patient: Some("pt-001".into()),
            actor: "demo patient".into(),
        },
        ("clinician", "learn-clinician") => Auth {
            role: Role::Clinician,
            patient: None,
            actor: "demo clinician".into(),
        },
        _ => return (StatusCode::UNAUTHORIZED, "invalid demo credentials").into_response(),
    };
    let id = format!("demo-{}", s.0.sessions.lock().unwrap().len() + 1);
    s.0.sessions.lock().unwrap().insert(
        id.clone(),
        Session {
            auth: a,
            last: Instant::now(),
        },
    );
    let mut o = (StatusCode::SEE_OTHER, [(header::LOCATION, "/")], "").into_response();
    o.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!("session={id}; HttpOnly; SameSite=Lax; Path=/")).unwrap(),
    );
    o
}
async fn home(State(s): State<App>, Extension(a): Extension<Auth>) -> Response {
    let patients: Vec<_> =
        s.0.data
            .patients
            .iter()
            .filter(|p| a.role == Role::Clinician || a.patient.as_deref() == Some(&p.id))
            .collect();
    let mut b = format!(
        "<p>Signed in as {}. Records are synthetic and this is not a production portal.</p>",
        a.actor
    );
    for p in patients {
        b.push_str(&format!(
            "<section><h2>{}</h2><p>{}</p><p>Next: {}</p></section>",
            p.name, p.summary, p.appointment
        ))
    }
    b.push_str(r#"<form method=post action=/messages><label>Message<textarea name=message required></textarea></label><button>Send message</button></form>"#);
    Html(page("Records and appointments", &b)).into_response()
}
#[derive(Deserialize)]
struct Message {
    message: String,
}
async fn message(
    State(s): State<App>,
    Extension(a): Extension<Auth>,
    Form(x): Form<Message>,
) -> Response {
    let owner = a.patient.unwrap_or_else(|| "clinician".into());
    s.0.messages.lock().unwrap().push((owner, x.message));
    Html(page("Message queued","<p>Message queued in this process. This learning app is not monitored for emergencies.</p>")).into_response()
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","synthetic_only":true}))
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/login", get(login_page).post(login))
        .route("/", get(home))
        .route("/messages", post(message))
        .layer(middleware::from_fn_with_state(s.clone(), auth))
        .layer(middleware::from_fn_with_state(s.clone(), audit))
        .with_state(s)
}
#[tokio::main]
async fn main() {
    let raw = std::env::var("SYNTHETIC_DATA")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| DATA.into());
    let s = App::load(&raw).expect("synthetic fixture required");
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
    fn sess(s: &App, r: Role, p: Option<&str>) -> String {
        let id = "test".into();
        s.0.sessions.lock().unwrap().insert(
            id,
            Session {
                auth: Auth {
                    role: r,
                    patient: p.map(str::to_owned),
                    actor: "test".into(),
                },
                last: Instant::now(),
            },
        );
        "session=test".into()
    }
    #[test]
    fn synthetic_only() {
        assert!(App::load(r#"{"notice":"real","patients":[]}"#).is_err())
    }
    #[tokio::test]
    async fn scope_and_message() {
        let s = App::load(DATA).unwrap();
        let c = sess(&s, Role::Patient, Some("pt-001"));
        let a = router(s.clone());
        let r = a
            .clone()
            .oneshot(
                Request::get("/")
                    .header(header::COOKIE, &c)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let b =
            String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
        assert!(b.contains("Avery Brooks") && !b.contains("Samira Cole"));
        let r = a
            .oneshot(
                Request::post("/messages")
                    .header(header::COOKIE, c)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("message=hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(s.0.messages.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn anonymous_denied() {
        let s = App::load(DATA).unwrap();
        assert_eq!(
            router(s)
                .oneshot(Request::get("/").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            401
        )
    }
}
