use axum::{
    extract::{Form, Query, Request, State},
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
    time::Instant,
};
const DATA: &str = include_str!("../../synthetic/dashboard-demo.json");
#[derive(Clone, Deserialize)]
struct Dataset {
    notice: String,
    encounters: Vec<Encounter>,
}
#[derive(Clone, Deserialize)]
struct Encounter {
    id: String,
    service: String,
    status: String,
    wait_minutes: u32,
}
struct Inner {
    data: Dataset,
    sessions: Mutex<HashMap<String, Instant>>,
    audit: Mutex<Vec<String>>,
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
            audit: Mutex::new(vec![]),
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
async fn auth(State(s): State<App>, r: Request, n: Next) -> Response {
    if matches!(r.uri().path(), "/health" | "/login") {
        return n.run(r).await;
    }
    let ok = cookie(r.headers()).is_some_and(|id| s.0.sessions.lock().unwrap().contains_key(&id));
    if !ok {
        return (
            StatusCode::UNAUTHORIZED,
            Html(page(
                "Sign in required",
                "<a href=/login>Demo staff sign in</a>",
            )),
        )
            .into_response();
    }
    n.run(r).await
}
async fn audit(State(s): State<App>, r: Request, n: Next) -> Response {
    let p = r.uri().path().to_owned();
    let o = n.run(r).await;
    if p != "/health" {
        let l=serde_json::json!({"event":"dashboard_view","path":p,"status":o.status().as_u16(),"synthetic":true}).to_string();
        println!("{l}");
        s.0.audit.lock().unwrap().push(l)
    }
    o
}
fn page(t: &str, b: &str) -> String {
    format!(
        r#"<!doctype html><html lang=en><meta name=viewport content="width=device-width"><title>{t}</title><style>body{{font:16px system-ui;margin:0;background:#f4f6fa;color:#182033}}nav{{padding:1rem 5%;background:#15233b;color:white}}main{{max-width:980px;margin:2rem auto;padding:1rem}}.cards{{display:flex;gap:1rem;flex-wrap:wrap}}.card,form,table{{background:white;padding:1rem;border-radius:12px;margin:1rem 0}}table{{width:100%;border-collapse:collapse}}th,td{{padding:.7rem;text-align:left;border-bottom:1px solid #ddd}}label,select,input,button{{margin:.4rem;padding:.5rem}}</style><nav aria-label="Dashboard">Clinical operations learning dashboard</nav><main><small>SYNTHETIC DATA · descriptive operations view · not clinical decision support</small><h1>{t}</h1>{b}<p>Security limitation: fixed demo staff credential and in-memory sessions; do not deploy with real data.</p></main></html>"#
    )
}
#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}
async fn login_page() -> Html<String> {
    Html(page(
        "Demo staff sign in",
        r#"<p>Demo-only credential: <code>staff / learn-dashboard</code>. It is not a real secret.</p><form method=post><label>Username<input name=username></label><label>Password<input type=password name=password></label><button>Sign in</button></form>"#,
    ))
}
async fn login(State(s): State<App>, Form(x): Form<Login>) -> Response {
    if (x.username.as_str(), x.password.as_str()) != ("staff", "learn-dashboard") {
        return (StatusCode::UNAUTHORIZED, "invalid demo credential").into_response();
    }
    let id = "staff-demo";
    s.0.sessions
        .lock()
        .unwrap()
        .insert(id.into(), Instant::now());
    let mut o = (StatusCode::SEE_OTHER, [(header::LOCATION, "/")], "").into_response();
    o.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("session=staff-demo; HttpOnly; SameSite=Lax; Path=/"),
    );
    o
}
#[derive(Default, Deserialize)]
struct Filter {
    service: Option<String>,
}
async fn home(State(s): State<App>, Query(f): Query<Filter>) -> Html<String> {
    let selected = f.service.as_deref().filter(|x| *x != "All");
    let rows: Vec<_> =
        s.0.data
            .encounters
            .iter()
            .filter(|e| selected.is_none_or(|x| e.service == x))
            .collect();
    let waiting = rows.iter().filter(|e| e.status == "waiting").count();
    let avg = if rows.is_empty() {
        0
    } else {
        rows.iter().map(|e| e.wait_minutes).sum::<u32>() / rows.len() as u32
    };
    let mut b = format!(
        r#"<form method=get><label>Service line<select name=service><option>All</option><option>Cardiology</option><option>Orthopedics</option><option>Primary Care</option></select></label><button>Apply filter</button></form><div class=cards><div class=card><b>{} encounters</b></div><div class=card><b>{waiting} waiting</b></div><div class=card><b>{avg} min average wait</b></div></div><table aria-label="Filtered synthetic encounters"><thead><tr><th>ID</th><th>Service</th><th>Status</th><th>Wait minutes</th></tr></thead><tbody>"#,
        rows.len()
    );
    for e in rows {
        b.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            e.id, e.service, e.status, e.wait_minutes
        ))
    }
    b.push_str("</tbody></table><p>Counts summarize this fixture only. No risk scores, diagnoses, recommendations, or automated alerts are produced.</p>");
    Html(page("Service flow metrics", &b))
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","synthetic_only":true}))
}
fn router(s: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/login", get(login_page).post(login))
        .route("/", get(home))
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
    #[test]
    fn rejects_real() {
        assert!(App::load(r#"{"notice":"real","encounters":[]}"#).is_err())
    }
    #[tokio::test]
    async fn filter_metrics() {
        let s = App::load(DATA).unwrap();
        s.0.sessions
            .lock()
            .unwrap()
            .insert("x".into(), Instant::now());
        let r = router(s)
            .oneshot(
                Request::get("/?service=Cardiology")
                    .header(header::COOKIE, "session=x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let b =
            String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
        assert!(
            b.contains("2 encounters")
                && b.contains("Cardiology")
                && !b.contains("<td>Orthopedics</td>")
        );
        assert!(b.contains("not clinical decision support"))
    }
    #[tokio::test]
    async fn auth_and_health() {
        let s = App::load(DATA).unwrap();
        let a = router(s);
        assert_eq!(
            a.clone()
                .oneshot(Request::get("/").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            a.oneshot(Request::get("/health").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            200
        )
    }
}
