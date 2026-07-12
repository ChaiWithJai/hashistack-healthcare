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
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const DATA: &str = include_str!("../../synthetic/outbound-followup-demo.json");

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
    contact: String,
    consented: bool,
    purpose: String,
}

#[derive(Clone)]
struct Followup {
    patient_id: String,
    message: String,
    status: String,
}

#[derive(Clone)]
struct Reply {
    patient_id: String,
    body: String,
    escalated: bool,
}

#[derive(Deserialize)]
struct QueueForm {
    patient_id: String,
    message: String,
}

#[derive(Deserialize)]
struct ReplyForm {
    patient_id: String,
    response: String,
}

struct Inner {
    data: Dataset,
    queue: Mutex<Vec<Followup>>,
    replies: Mutex<Vec<Reply>>,
    opted_out: Mutex<HashSet<String>>,
    sessions: Mutex<HashMap<String, Instant>>,
    audit: Mutex<Vec<String>>,
    idle: Duration,
}

#[derive(Clone)]
struct App(Arc<Inner>);

impl App {
    fn load(raw: &str, idle: Duration) -> Result<Self, String> {
        let data: Dataset = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if !data.notice.contains("SYNTHETIC") {
            return Err("refusing to boot: dataset is not marked SYNTHETIC".into());
        }
        Ok(Self(Arc::new(Inner {
            data,
            queue: Mutex::new(Vec::new()),
            replies: Mutex::new(Vec::new()),
            opted_out: Mutex::new(HashSet::new()),
            sessions: Mutex::new(HashMap::new()),
            audit: Mutex::new(Vec::new()),
            idle,
        })))
    }
}

fn cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| part.trim().strip_prefix("session=").map(str::to_owned))
}

async fn session(State(state): State<App>, request: Request, next: Next) -> Response {
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }
    if let Some(id) = cookie(request.headers()) {
        let expired = {
            let mut sessions = state.0.sessions.lock().unwrap();
            let expired = sessions
                .get(&id)
                .is_none_or(|last_seen| last_seen.elapsed() > state.0.idle);
            if expired {
                sessions.remove(&id);
            } else {
                sessions.insert(id, Instant::now());
            }
            expired
        };
        if expired {
            return (
                StatusCode::UNAUTHORIZED,
                Html(page(
                    "Session expired",
                    "<p>Your demo session ended after inactivity. Reload to start a new synthetic session.</p>",
                )),
            )
                .into_response();
        }
        return next.run(request).await;
    }
    let id = format!("demo-{}", state.0.sessions.lock().unwrap().len() + 1);
    state
        .0
        .sessions
        .lock()
        .unwrap()
        .insert(id.clone(), Instant::now());
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!("session={id}; Path=/; HttpOnly; SameSite=Lax")).unwrap(),
    );
    response
}

async fn audit(State(state): State<App>, request: Request, next: Next) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    let line = serde_json::json!({
        "control":"audit-log", "method":method, "path":path,
        "status":response.status().as_u16(), "dataset":"SYNTHETIC"
    })
    .to_string();
    println!("{line}");
    state.0.audit.lock().unwrap().push(line);
    response
}

fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title><style>body{{font:17px system-ui;line-height:1.5;max-width:960px;margin:2rem auto;padding:1rem;background:#f4f7fb;color:#172033}}main{{background:white;padding:2rem;border-radius:16px;box-shadow:0 8px 30px #18243a18}}.notice{{border-left:5px solid #a13b00;background:#fff3e8;padding:1rem}}.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(290px,1fr));gap:1.5rem}}label,input,textarea,button{{display:block;width:100%;box-sizing:border-box}}input,textarea,button{{font:inherit;padding:.7rem;margin:.35rem 0 1rem}}button{{width:auto;background:#183d70;color:white;border:0;border-radius:8px}}:focus-visible{{outline:3px solid #e76f00;outline-offset:2px}}table{{width:100%;border-collapse:collapse}}th,td{{text-align:left;padding:.5rem;border-bottom:1px solid #ccd3df}}.urgent{{color:#9b1c1c;font-weight:700}}</style></head><body><main><p><strong>SYNTHETIC TRAINING APP</strong> · no messages leave this process</p><h1>{title}</h1><div class="notice" role="note"><strong>Limits:</strong> Demo-cookie sessions are not production authentication. Confirm documented consent before contact. This is not an emergency service and does not monitor replies continuously. Call local emergency services for urgent danger.</div>{body}</main></body></html>"#
    )
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","synthetic_only":true}))
}

async fn home(State(state): State<App>) -> Html<String> {
    let patients = state
        .0
        .data
        .patients
        .iter()
        .map(|patient| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc(&patient.id),
                esc(&patient.name),
                esc(&patient.contact),
                if patient.consented {
                    "documented"
                } else {
                    "NOT documented"
                },
                esc(&patient.purpose)
            )
        })
        .collect::<String>();
    let queue = state
        .0
        .queue
        .lock()
        .unwrap()
        .iter()
        .map(|item| {
            format!(
                "<li><strong>{}</strong>: {} — {}</li>",
                esc(&item.patient_id),
                esc(&item.message),
                esc(&item.status)
            )
        })
        .collect::<String>();
    let replies = state
        .0
        .replies
        .lock()
        .unwrap()
        .iter()
        .map(|reply| {
            format!(
                "<li class={}><strong>{}</strong>: {}{}</li>",
                if reply.escalated { "urgent" } else { "normal" },
                esc(&reply.patient_id),
                esc(&reply.body),
                if reply.escalated {
                    " — CLINICIAN ESCALATION"
                } else {
                    ""
                }
            )
        })
        .collect::<String>();
    Html(page(
        "Outbound follow-up queue",
        &format!(
            r#"<h2>Synthetic patient consent register</h2><table><thead><tr><th>ID</th><th>Patient</th><th>Masked contact</th><th>Consent</th><th>Approved purpose</th></tr></thead><tbody>{patients}</tbody></table><div class="grid"><section><h2>Queue approved follow-up</h2><form method="post" action="/queue"><label for="contact-id">Patient ID to contact</label><input id="contact-id" name="patient_id" required><label for="message">Approved follow-up message</label><textarea id="message" name="message" required></textarea><button type="submit">Queue follow-up</button></form></section><section><h2>Record a simulated response</h2><form method="post" action="/responses"><label for="response-id">Patient ID responding</label><input id="response-id" name="patient_id" required><label for="response">Patient response</label><textarea id="response" name="response" required></textarea><button type="submit">Record response</button></form></section></div><h2>Follow-up queue</h2><ul>{queue}</ul><h2>Responses and clinician inbox</h2><ul>{replies}</ul>"#
        ),
    ))
}

async fn queue(State(state): State<App>, Form(form): Form<QueueForm>) -> Response {
    let patient = state
        .0
        .data
        .patients
        .iter()
        .find(|p| p.id == form.patient_id);
    let Some(patient) = patient else {
        return (StatusCode::BAD_REQUEST, "Unknown synthetic patient").into_response();
    };
    if !patient.consented || state.0.opted_out.lock().unwrap().contains(&form.patient_id) {
        return (
            StatusCode::CONFLICT,
            Html(page("Follow-up blocked", "<p>No current documented consent. Nothing was queued or sent.</p><p><a href=\"/\">Back</a></p>")),
        ).into_response();
    }
    if form.message.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Message is required").into_response();
    }
    state.0.queue.lock().unwrap().push(Followup {
        patient_id: form.patient_id,
        message: form.message,
        status: "queued locally; NOT sent".into(),
    });
    Html(page("Follow-up queued", "<p>Queued inside this synthetic demo. No SMS, email, or patient-system call was made.</p><p><a href=\"/\">Back to queue</a></p>")).into_response()
}

fn concerning(body: &str) -> bool {
    let normalized = body.to_ascii_lowercase();
    [
        "cannot breathe",
        "chest pain",
        "severe pain",
        "much worse",
        "bleeding",
        "suicidal",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

async fn respond(State(state): State<App>, Form(form): Form<ReplyForm>) -> Response {
    if !state
        .0
        .data
        .patients
        .iter()
        .any(|p| p.id == form.patient_id)
    {
        return (StatusCode::BAD_REQUEST, "Unknown synthetic patient").into_response();
    }
    if !state
        .0
        .queue
        .lock()
        .unwrap()
        .iter()
        .any(|item| item.patient_id == form.patient_id)
    {
        return (
            StatusCode::CONFLICT,
            "No queued follow-up exists for this synthetic patient",
        )
            .into_response();
    }
    let trimmed = form.response.trim();
    if trimmed.is_empty() {
        return (StatusCode::BAD_REQUEST, "Response is required").into_response();
    }
    let stop = trimmed.eq_ignore_ascii_case("stop");
    if stop {
        state
            .0
            .opted_out
            .lock()
            .unwrap()
            .insert(form.patient_id.clone());
    }
    let escalated = concerning(trimmed);
    state.0.replies.lock().unwrap().push(Reply {
        patient_id: form.patient_id,
        body: trimmed.to_string(),
        escalated,
    });
    let outcome = if stop {
        "Consent withdrawn. Future follow-up is blocked."
    } else if escalated {
        "Escalated to clinician inbox. This demo does not provide emergency monitoring."
    } else {
        "Response recorded for staff review."
    };
    Html(page(
        "Response recorded",
        &format!(
            "<p class=\"{}\">{outcome}</p><p><a href=\"/\">Back to queue</a></p>",
            if escalated { "urgent" } else { "normal" }
        ),
    ))
    .into_response()
}

fn router(state: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(home))
        .route("/queue", post(queue))
        .route("/responses", post(respond))
        .layer(middleware::from_fn_with_state(state.clone(), session))
        .layer(middleware::from_fn_with_state(state.clone(), audit))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let raw = std::env::var("SYNTHETIC_DATA")
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_else(|| DATA.into());
    let state = App::load(&raw, Duration::from_secs(900)).expect("synthetic seed required");
    let bind = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    axum::serve(listener, router(state)).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::ServiceExt;

    async fn post(app: Router, path: &str, body: &str) -> (StatusCode, String) {
        let response = app
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        (status, body)
    }

    #[test]
    fn refuses_unmarked_data() {
        assert!(App::load(
            r#"{"dataset":"real","notice":"patient export","patients":[]}"#,
            Duration::ZERO
        )
        .is_err());
    }

    #[tokio::test]
    async fn consent_queue_response_escalation_and_opt_out_work() {
        let state = App::load(DATA, Duration::from_secs(60)).unwrap();
        let app = router(state.clone());
        let (status, body) =
            post(app.clone(), "/queue", "patient_id=fu-003&message=Reminder").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body.contains("Nothing was queued"));
        assert!(state.0.queue.lock().unwrap().is_empty());

        let (status, _) = post(
            app.clone(),
            "/queue",
            "patient_id=fu-001&message=How+are+you%3F",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(state.0.queue.lock().unwrap().len(), 1);

        let (status, body) = post(
            app.clone(),
            "/responses",
            "patient_id=fu-001&response=My+pain+is+much+worse+and+I+cannot+breathe",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Escalated to clinician inbox"));
        assert!(state.0.replies.lock().unwrap()[0].escalated);

        let (status, _) = post(app.clone(), "/responses", "patient_id=fu-001&response=STOP").await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = post(app, "/queue", "patient_id=fu-001&message=Another+message").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(state.0.audit.lock().unwrap().len() >= 5);
    }

    #[tokio::test]
    async fn home_is_accessible_and_honest() {
        let state = App::load(DATA, Duration::from_secs(60)).unwrap();
        let response = router(state)
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        for marker in [
            "<main>",
            "lang=\"en\"",
            "Patient ID to contact",
            "not production authentication",
            "not an emergency service",
            "no messages leave this process",
        ] {
            assert!(body.contains(marker), "missing {marker}");
        }
    }
}
