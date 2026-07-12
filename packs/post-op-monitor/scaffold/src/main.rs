//! post-op monitor — the pack's runnable scaffold (issue #5 pattern-setter).
//!
//! This is the app template the pack ships: what the agent scaffolds before
//! the doctor's first conversational edit, and what the ejection bundle
//! carries under `app/`. One file, the platform's own dependency set
//! (axum/tokio/serde), and every control it claims is really in the code:
//!
//! - pain + wound check-in form, server-rendered in the same sketchy
//!   wireframe skin as the platform UI (`web/index.html`)
//! - photo upload stub: multipart accepted, held in memory, honestly
//!   labeled NOT yet encrypted at rest (hipaa-core + Vault transit TODO)
//! - audit middleware: every data-touching request becomes one JSONL line
//!   on stdout — the hipaa-core *placeholder*, labeled as such in each line
//! - auto-logoff: session cookie with an idle timeout (gate `auto-logoff`)
//! - escalation flags: check-ins over threshold route to the practice
//!   inbox (pack gate semantics — see ../gates/README.md)
//! - synthetic seed loaded at boot from ../synthetic/post-op-demo.json;
//!   the app refuses any dataset not marked SYNTHETIC
//!
//! PHI inventory (#3): fields that would hold protected health information
//! in a real deployment carry a `// phi:` marker, and each struct holding
//! them declares its encryption disposition with `// phi-encryption:` —
//! `stub` today (everything lives in memory over synthetic data; hipaa-core
//! encryptField via Vault transit is the labeled TODO). The platform's
//! phi-encryption gate reads these markers as evidence and reports the stub
//! as `stubbed`, never as a pass.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Form, Multipart, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

/// Compile-time copy of the pack's synthetic seed. The boot path prefers the
/// file on disk (`../synthetic/post-op-demo.json`, or `$SYNTHETIC_DATA`) so
/// edits to the dataset show up without a rebuild; this embedded copy is the
/// fallback that keeps the ejected app bootable anywhere.
const EMBEDDED_DATASET: &str = include_str!("../../synthetic/post-op-demo.json");

/// Pain at or above this routes a flag to the practice inbox (0–10 scale).
const PAIN_ESCALATION_THRESHOLD: u8 = 7;

/// Wound statuses a patient may report.
const KNOWN_WOUND_STATUSES: &[&str] = &[
    "clean",
    "redness",
    "swelling",
    "drainage",
    "opening",
    "spreading-redness",
];

/// Wound statuses that route a flag to the practice inbox regardless of pain.
const CONCERNING_WOUND_STATUSES: &[&str] = &["drainage", "opening", "spreading-redness"];

// ---------- synthetic seed ----------

#[derive(Clone, Debug, Deserialize)]
struct Dataset {
    dataset: String,
    notice: String,
    patients: Vec<Patient>,
}

#[derive(Clone, Debug, Deserialize)]
struct Patient {
    // phi-encryption: stub — in-memory over the synthetic seed only;
    // hipaa-core encryptField via Vault transit before any real storage.
    id: String,
    name: String,         // phi: patient name
    age: u8,              // phi: age
    procedure: String,    // phi: procedure performed
    surgery_date: String, // phi: date of surgery
    surgeon: String,      // phi: treating surgeon
    #[serde(default)]
    checkins: Vec<SeedCheckin>,
}

#[derive(Clone, Debug, Deserialize)]
struct SeedCheckin {
    // phi-encryption: stub — rides inside Patient, same labeled TODO.
    day: u32,
    pain: u8,      // phi: reported pain score
    wound: String, // phi: reported wound status
    note: String,  // phi: free-text clinical note
}

// ---------- runtime state ----------

#[derive(Clone, Debug)]
struct Checkin {
    // phi-encryption: stub — held in memory only; hipaa-core encryptField
    // via Vault transit before any real storage backend is wired.
    patient_id: String, // phi: patient identifier
    pain: u8,           // phi: reported pain score
    wound: String,      // phi: reported wound status
    note: String,       // phi: free-text clinical note
    at: u64,
}

#[derive(Clone, Debug)]
struct PhotoStub {
    // phi-encryption: stub — see encrypted_at_rest below; the label is the
    // whole point of this struct.
    patient_id: String, // phi: patient identifier
    filename: String,   // phi: wound photo filename
    bytes: usize,
    /// Honest label: photos are held in memory and NOT encrypted at rest.
    /// TODO(hipaa-core): encryptField via Vault transit before any real
    /// storage backend is wired (see ../policies/vault-policy.hcl).
    encrypted_at_rest: bool,
}

/// A check-in over threshold, routed to the practice inbox — the pack's
/// escalation-path semantics (../gates/README.md).
#[derive(Clone, Debug)]
struct Flag {
    patient_id: String,
    reason: String,
    at: u64,
}

/// Where audit JSONL lines go: stdout in production, memory in tests so the
/// contract "a check-in produces an audit line" is assertable.
enum AuditSink {
    Stdout,
    /// Constructed by the test suite only — the running app always streams
    /// to stdout, where an operator (or a log shipper) can see it.
    #[cfg_attr(not(test), allow(dead_code))]
    Memory(Mutex<Vec<String>>),
}

impl AuditSink {
    fn write(&self, line: String) {
        match self {
            AuditSink::Stdout => println!("{line}"),
            AuditSink::Memory(buffer) => buffer.lock().unwrap().push(line),
        }
    }
}

struct Inner {
    dataset_name: String,
    patients: Vec<Patient>,
    checkins: Mutex<Vec<Checkin>>,
    photos: Mutex<Vec<PhotoStub>>,
    inbox: Mutex<Vec<Flag>>,
    sessions: Mutex<HashMap<String, Instant>>,
    session_seq: AtomicU64,
    idle_timeout: Duration,
    audit: AuditSink,
}

#[derive(Clone)]
struct AppState(Arc<Inner>);

impl AppState {
    /// Parse a seed and refuse anything not marked synthetic. The scaffold
    /// is sandbox software; it must be impossible to boot it over data that
    /// doesn't carry the SYNTHETIC notice.
    fn from_dataset(json: &str, idle_timeout: Duration, audit: AuditSink) -> Result<Self, String> {
        let dataset: Dataset =
            serde_json::from_str(json).map_err(|e| format!("seed dataset is not valid: {e}"))?;
        if !dataset.notice.contains("SYNTHETIC") {
            return Err(format!(
                "refusing to boot: dataset {:?} is not marked SYNTHETIC — this scaffold only ever sees synthetic data",
                dataset.dataset
            ));
        }
        Ok(Self(Arc::new(Inner {
            dataset_name: dataset.dataset,
            patients: dataset.patients,
            checkins: Mutex::new(Vec::new()),
            photos: Mutex::new(Vec::new()),
            inbox: Mutex::new(Vec::new()),
            sessions: Mutex::new(HashMap::new()),
            session_seq: AtomicU64::new(0),
            idle_timeout,
            audit,
        })))
    }

    fn mint_session(&self) -> String {
        let seq = self.0.session_seq.fetch_add(1, Ordering::Relaxed);
        let id = format!("s{:04}-{}", seq, unix_now());
        self.0
            .sessions
            .lock()
            .unwrap()
            .insert(id.clone(), Instant::now());
        id
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------- middleware ----------

fn session_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (key, value) = pair.trim().split_once('=')?;
        (key == "session").then(|| value.to_string())
    })
}

/// Auto-logoff (gate `auto-logoff`): every session cookie carries an idle
/// clock; a request after the timeout is refused, its session destroyed,
/// and the cookie cleared. `/health` is exempt — probes are not sessions.
async fn auto_logoff(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }
    if let Some(id) = session_from_headers(request.headers()) {
        let expired = {
            let mut sessions = state.0.sessions.lock().unwrap();
            match sessions.get(&id) {
                Some(last_seen) if last_seen.elapsed() > state.0.idle_timeout => {
                    sessions.remove(&id);
                    true
                }
                Some(_) => {
                    sessions.insert(id, Instant::now());
                    false
                }
                // Unknown or stale cookie (e.g. the process restarted):
                // same treatment as idle — a fresh session must be minted.
                None => true,
            }
        };
        if expired {
            return logged_off();
        }
        return next.run(request).await;
    }
    // First touch: mint a session so the idle clock starts now.
    let id = state.mint_session();
    let mut response = next.run(request).await;
    let cookie = format!("session={id}; Path=/; HttpOnly; SameSite=Lax");
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

fn logged_off() -> Response {
    let body = page(
        "logged off",
        "<div class=\"sk pad\"><b>You were logged off after inactivity.</b>\
         <p>The auto-logoff control ended this session (idle timeout). Reload the page to start a new one.</p></div>"
            .to_string(),
    );
    let mut response = (StatusCode::UNAUTHORIZED, Html(body)).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("session=; Path=/; Max-Age=0"),
    );
    response
}

/// Audit middleware — the hipaa-core PLACEHOLDER, and labeled as such in
/// every line it writes. Every data-touching request (everything except the
/// `/health` probe) becomes one JSONL line on stdout: who (session), what
/// (method + path), when, and how it ended. The real hipaa-core library
/// replaces the sink, not the contract.
async fn audit_jsonl(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    if path == "/health" {
        return next.run(request).await;
    }
    let method = request.method().to_string();
    let actor = session_from_headers(request.headers()).unwrap_or_else(|| "anonymous".to_string());
    let response = next.run(request).await;
    let line = serde_json::json!({
        "at": unix_now(),
        "actor": actor,
        "action": "http.request",
        "method": method,
        "path": path,
        "status": response.status().as_u16(),
        "control": "audit-log",
        "note": "hipaa-core placeholder — stdout JSONL until the shared audit library lands",
    });
    state.0.audit.write(line.to_string());
    response
}

// ---------- handlers ----------

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "app": "post-op-monitor-scaffold" }))
}

async fn home(State(state): State<AppState>) -> Html<String> {
    let mut body = String::new();
    body.push_str(&format!(
        "<div class=\"sk pad\"><b>recovery check-in</b> <span class=\"muted\">seeded from {} — synthetic data only</span>\
         <form method=\"post\" action=\"/checkin\" class=\"col\">\
         <label>patient <select name=\"patient_id\">",
        esc(&state.0.dataset_name)
    ));
    for patient in &state.0.patients {
        body.push_str(&format!(
            "<option value=\"{}\">{}, {} — {} ({}, {})</option>",
            esc(&patient.id),
            esc(&patient.name),
            patient.age,
            esc(&patient.procedure),
            esc(&patient.surgery_date),
            esc(&patient.surgeon)
        ));
    }
    body.push_str(
        "</select></label>\
         <label>pain (0–10) <input type=\"number\" name=\"pain\" min=\"0\" max=\"10\" value=\"3\"></label>\
         <label>wound looks <select name=\"wound\">",
    );
    for status in KNOWN_WOUND_STATUSES {
        body.push_str(&format!("<option value=\"{status}\">{status}</option>"));
    }
    body.push_str(
        "</select></label>\
         <label>note <input type=\"text\" name=\"note\" placeholder=\"how is recovery going?\"></label>\
         <button class=\"b bp\" type=\"submit\">log today&rsquo;s check-in</button>\
         </form></div>\
         <div class=\"sk pad\"><b>wound photo</b>\
         <form method=\"post\" action=\"/photos\" enctype=\"multipart/form-data\" class=\"col\">\
         <input type=\"hidden\" name=\"patient_id\" value=\"pt-001\">\
         <input type=\"file\" name=\"photo\">\
         <button class=\"b\" type=\"submit\">upload</button>\
         <span class=\"note\">held in memory; encryption at rest is a labeled TODO (hipaa-core + Vault transit)</span>\
         </form></div>",
    );

    let checkins = state.0.checkins.lock().unwrap();
    body.push_str(&format!(
        "<div class=\"sk pad\"><b>check-ins this session</b> <span class=\"muted\">{}</span><ul>",
        checkins.len()
    ));
    for checkin in checkins.iter().rev().take(10) {
        body.push_str(&format!(
            "<li>{} — pain {}/10, wound {} — {} <span class=\"muted\">at {}</span></li>",
            esc(&checkin.patient_id),
            checkin.pain,
            esc(&checkin.wound),
            esc(&checkin.note),
            checkin.at
        ));
    }
    drop(checkins);
    body.push_str("</ul></div>");

    let inbox = state.0.inbox.lock().unwrap();
    body.push_str(&format!(
        "<div class=\"sk pad\"><b>practice inbox — escalation flags</b> <span class=\"muted\">{}</span><ul>",
        inbox.len()
    ));
    for flag in inbox.iter().rev().take(10) {
        body.push_str(&format!(
            "<li class=\"note\">{} — {} <span class=\"muted\">at {}</span></li>",
            esc(&flag.patient_id),
            esc(&flag.reason),
            flag.at
        ));
    }
    drop(inbox);
    body.push_str("</ul></div>");

    body.push_str("<div class=\"sk pad\"><b>recovery histories (synthetic seed)</b><ul>");
    for patient in &state.0.patients {
        if let Some(latest) = patient.checkins.last() {
            body.push_str(&format!(
                "<li>{} — day {}: pain {}/10, wound {} — {}</li>",
                esc(&patient.name),
                latest.day,
                latest.pain,
                esc(&latest.wound),
                esc(&latest.note)
            ));
        }
    }
    body.push_str("</ul></div>");

    body.push_str(&format!(
        "<div class=\"sk pad muted\">{} synthetic patients seeded · every request above lands in the stdout audit log (JSONL) · sessions auto-logoff after idle</div>",
        state.0.patients.len()
    ));
    Html(page("post-op monitor", body))
}

#[derive(Deserialize)]
struct CheckinForm {
    patient_id: String,
    pain: u8,
    wound: String,
    #[serde(default)]
    note: String,
}

async fn checkin(State(state): State<AppState>, Form(form): Form<CheckinForm>) -> Response {
    let Some(patient) = state.0.patients.iter().find(|p| p.id == form.patient_id) else {
        return (
            StatusCode::NOT_FOUND,
            Html(page(
                "unknown patient",
                "<div class=\"sk pad\">no such patient in the synthetic seed</div>".to_string(),
            )),
        )
            .into_response();
    };
    if form.pain > 10 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Html(page(
                "bad check-in",
                "<div class=\"sk pad\">pain must be on the 0–10 scale</div>".to_string(),
            )),
        )
            .into_response();
    }
    if !KNOWN_WOUND_STATUSES.contains(&form.wound.as_str()) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Html(page(
                "bad check-in",
                "<div class=\"sk pad\">unknown wound status</div>".to_string(),
            )),
        )
            .into_response();
    }

    state.0.checkins.lock().unwrap().push(Checkin {
        patient_id: form.patient_id.clone(),
        pain: form.pain,
        wound: form.wound.clone(),
        note: form.note.clone(),
        at: unix_now(),
    });

    // Escalation-path semantics (../gates/README.md): a flag over threshold
    // must route to the practice inbox — not a dashboard nobody watches.
    let mut reasons = Vec::new();
    if form.pain >= PAIN_ESCALATION_THRESHOLD {
        reasons.push(format!(
            "pain {}/10 at or over threshold {PAIN_ESCALATION_THRESHOLD}",
            form.pain
        ));
    }
    if CONCERNING_WOUND_STATUSES.contains(&form.wound.as_str()) {
        reasons.push(format!("wound reported as {:?}", form.wound));
    }
    let flagged = !reasons.is_empty();
    if flagged {
        state.0.inbox.lock().unwrap().push(Flag {
            patient_id: form.patient_id.clone(),
            reason: reasons.join("; "),
            at: unix_now(),
        });
    }

    let confirmation = if flagged {
        format!(
            "<div class=\"sk pad\"><b>check-in recorded for {}</b>\
             <p class=\"note\">flag routed to the practice inbox: {}</p>\
             <a href=\"/\">back</a></div>",
            esc(&patient.name),
            esc(&reasons.join("; "))
        )
    } else {
        format!(
            "<div class=\"sk pad\"><b>check-in recorded for {}</b>\
             <p>within expected recovery range — no escalation.</p>\
             <a href=\"/\">back</a></div>",
            esc(&patient.name)
        )
    };
    Html(page("check-in recorded", confirmation)).into_response()
}

/// Photo upload STUB: multipart is accepted and counted, the bytes are held
/// in memory, and the record says plainly that encryption at rest has not
/// happened yet. The real path is hipaa-core encryptField via Vault transit
/// (../policies/vault-policy.hcl) before any storage backend is wired.
async fn photos(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut patient_id = "unspecified".to_string();
    let mut stored: Option<PhotoStub> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        let filename = field.file_name().unwrap_or("upload.bin").to_string();
        let Ok(bytes) = field.bytes().await else {
            continue;
        };
        if name == "patient_id" {
            patient_id = String::from_utf8_lossy(&bytes).to_string();
        } else {
            stored = Some(PhotoStub {
                patient_id: patient_id.clone(),
                filename,
                bytes: bytes.len(),
                encrypted_at_rest: false, // labeled TODO, never silently claimed
            });
        }
    }
    let Some(mut photo) = stored else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Html(page(
                "no photo",
                "<div class=\"sk pad\">multipart request carried no file field</div>".to_string(),
            )),
        )
            .into_response();
    };
    photo.patient_id = patient_id;
    // The label is computed from the record, so the UI can never claim a
    // control the stored object doesn't carry.
    let storage_label = if photo.encrypted_at_rest {
        "encrypted at rest (hipaa-core field encryption)".to_string()
    } else {
        "held in memory only — NOT encrypted at rest yet. \
         TODO(hipaa-core): encryptField via Vault transit before real storage."
            .to_string()
    };
    let note = format!(
        "<div class=\"sk pad\"><b>photo received: {} ({} bytes) for {}</b>\
         <p class=\"note\">{storage_label}</p>\
         <a href=\"/\">back</a></div>",
        esc(&photo.filename),
        photo.bytes,
        esc(&photo.patient_id)
    );
    state.0.photos.lock().unwrap().push(photo);
    Html(page("photo received", note)).into_response()
}

// ---------- presentation: the platform's sketchy wireframe skin ----------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(title: &str, body: String) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title>\
         <link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">\
         <link href=\"https://fonts.googleapis.com/css2?family=Patrick+Hand&family=Shantell+Sans:wght@400;600&display=swap\" rel=\"stylesheet\">\
         <style>\
         /* Sketchy wireframe kit — same skin as the platform's web/index.html, \
            so the scaffold and the storyboards read as the same object. */\
         *{{box-sizing:border-box}}\
         body{{margin:0;background:#f0eee9;font-family:'Shantell Sans',system-ui,sans-serif;color:#2b2b2b;font-size:14px}}\
         header{{display:flex;align-items:center;gap:14px;padding:12px 20px;border-bottom:1px solid rgba(0,0,0,.08)}}\
         header h1{{font-size:15px;margin:0}}\
         header .who{{margin-left:auto;color:#888;font-size:12px}}\
         main{{max-width:760px;margin:22px auto;padding:0 20px;display:flex;flex-direction:column;gap:12px;font-family:'Patrick Hand',cursive;font-size:15px}}\
         .sk{{border:1.4px solid #2b2b2b;border-radius:225px 6px 255px 6px/6px 255px 6px 225px;background:#fdfdfb}}\
         .pad{{padding:12px 16px}}\
         .col{{display:flex;flex-direction:column;gap:8px;margin-top:8px}}\
         label{{display:flex;flex-direction:column;gap:3px}}\
         input,select{{font-family:'Patrick Hand',cursive;font-size:15px;border:1.4px solid #2b2b2b;border-radius:225px 6px 255px 6px/6px 255px 6px 225px;background:#fff;padding:6px 10px}}\
         .b{{display:inline-flex;align-items:center;justify-content:center;gap:5px;border:1.4px solid #2b2b2b;border-radius:6px;padding:4px 11px;background:#fff;font-family:'Patrick Hand',cursive;font-size:14px;cursor:pointer}}\
         .bp{{background:#2a78d6;color:#fff;border-color:#1c5aa8}}\
         .note{{font-family:'Patrick Hand',cursive;font-size:13px;color:#8a5a14}}\
         .muted{{color:#888;font-size:12px;font-family:'Shantell Sans',sans-serif}}\
         a{{color:#2a78d6}}\
         footer{{max-width:760px;margin:10px auto 40px;padding:0 20px;color:#999;font-size:11.5px}}\
         </style></head><body>\
         <header><h1>post-op monitor</h1><span class=\"who\">pack scaffold · synthetic data only</span></header>\
         <main>{body}</main>\
         <footer>Runnable scaffold from pack post-op-monitor (issue #5). Audit log: stdout JSONL (hipaa-core placeholder). Photos: in-memory stub, encryption at rest TODO.</footer>\
         </body></html>"
    )
}

// ---------- boot ----------

fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/checkin", post(checkin))
        .route("/photos", post(photos))
        .route("/health", get(health))
        // Layer order: last added runs first, so audit wraps auto-logoff —
        // even a logged-off attempt leaves an audit line.
        .layer(middleware::from_fn_with_state(state.clone(), auto_logoff))
        .layer(middleware::from_fn_with_state(state.clone(), audit_jsonl))
        .with_state(state)
}

/// Prefer the seed on disk (edits show up without a rebuild); fall back to
/// the compile-time copy so the ejected app boots from anywhere.
fn load_dataset() -> (String, String) {
    let path = std::env::var("SYNTHETIC_DATA")
        .unwrap_or_else(|_| "../synthetic/post-op-demo.json".to_string());
    match std::fs::read_to_string(&path) {
        Ok(json) => (json, path),
        Err(_) => (
            EMBEDDED_DATASET.to_string(),
            "embedded compile-time copy".to_string(),
        ),
    }
}

#[tokio::main]
async fn main() {
    let idle_secs = std::env::var("AUTO_LOGOFF_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(900u64);
    let (json, source) = load_dataset();
    let state = AppState::from_dataset(&json, Duration::from_secs(idle_secs), AuditSink::Stdout)
        .expect("synthetic seed must parse and carry the SYNTHETIC notice");
    println!(
        "post-op monitor scaffold: {} synthetic patients from {source}; auto-logoff after {idle_secs}s idle",
        state.0.patients.len()
    );
    let bind = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {bind}: {e}"));
    println!("listening on http://{bind}");
    axum::serve(listener, app(state)).await.expect("serve");
}

// ---------- tests: the scaffold's own contract ----------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use tower::util::ServiceExt;

    fn test_state(idle_timeout: Duration) -> AppState {
        AppState::from_dataset(
            EMBEDDED_DATASET,
            idle_timeout,
            AuditSink::Memory(Mutex::new(Vec::new())),
        )
        .expect("embedded seed parses")
    }

    fn audit_lines(state: &AppState) -> Vec<String> {
        match &state.0.audit {
            AuditSink::Memory(buffer) => buffer.lock().unwrap().clone(),
            AuditSink::Stdout => panic!("test state uses the memory sink"),
        }
    }

    async fn body_text(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn health_is_up_and_exempt_from_audit_and_sessions() {
        let state = test_state(Duration::from_secs(900));
        let response = app(state.clone())
            .oneshot(HttpRequest::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
        assert!(body_text(response).await.contains("\"status\":\"ok\""));
        assert!(audit_lines(&state).is_empty(), "probes are not data access");
    }

    #[tokio::test]
    async fn seed_loads_twelve_synthetic_patients_and_refuses_unmarked_data() {
        let state = test_state(Duration::from_secs(900));
        assert_eq!(state.0.patients.len(), 12);
        // Sanity: every synthetic patient is fully formed, histories typed.
        for patient in &state.0.patients {
            assert!(patient.age > 0 && !patient.name.is_empty() && !patient.procedure.is_empty());
            assert!(!patient.checkins.is_empty(), "{} has history", patient.id);
            for entry in &patient.checkins {
                assert!(entry.day >= 1 && entry.pain <= 10 && !entry.note.is_empty());
                assert!(
                    KNOWN_WOUND_STATUSES.contains(&entry.wound.as_str()),
                    "seed uses only statuses the form can report: {}",
                    entry.wound
                );
            }
        }

        let unmarked = r#"{"dataset":"suspicious","notice":"totally real data","patients":[]}"#;
        match AppState::from_dataset(unmarked, Duration::from_secs(1), AuditSink::Stdout) {
            Ok(_) => panic!("data without the SYNTHETIC notice must be refused"),
            Err(err) => assert!(err.contains("refusing to boot")),
        }
    }

    #[tokio::test]
    async fn checkin_over_threshold_flags_inbox_and_writes_an_audit_jsonl_line() {
        let state = test_state(Duration::from_secs(900));
        let request = HttpRequest::post("/checkin")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(
                "patient_id=pt-001&pain=8&wound=drainage&note=dressing+soaked+through",
            ))
            .unwrap();
        let response = app(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        assert!(
            body.contains("practice inbox"),
            "escalation is visible: {body}"
        );

        assert_eq!(state.0.checkins.lock().unwrap().len(), 1);
        let inbox = state.0.inbox.lock().unwrap();
        assert_eq!(inbox.len(), 1, "pain 8 + drainage routes one flag");
        assert!(inbox[0].reason.contains("pain 8/10"));
        assert!(inbox[0].reason.contains("drainage"));
        drop(inbox);

        let lines = audit_lines(&state);
        assert_eq!(lines.len(), 1, "one data-touching request, one JSONL line");
        let event: serde_json::Value = serde_json::from_str(&lines[0]).expect("line is JSON");
        assert_eq!(event["path"], "/checkin");
        assert_eq!(event["method"], "POST");
        assert_eq!(event["status"], 200);
        assert_eq!(event["control"], "audit-log");
        assert!(
            event["note"].as_str().unwrap().contains("placeholder"),
            "the hipaa-core stand-in labels itself honestly"
        );
    }

    #[tokio::test]
    async fn in_range_checkin_does_not_flag() {
        let state = test_state(Duration::from_secs(900));
        let request = HttpRequest::post("/checkin")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(
                "patient_id=pt-003&pain=2&wound=clean&note=all+good",
            ))
            .unwrap();
        let response = app(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.0.inbox.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn idle_session_is_logged_off() {
        // Zero idle timeout: any second touch is past the deadline.
        let state = test_state(Duration::ZERO);

        let first = app(state.clone())
            .oneshot(HttpRequest::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let cookie = first
            .headers()
            .get(header::SET_COOKIE)
            .expect("first touch mints a session")
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert!(cookie.starts_with("session="));

        let second = app(state.clone())
            .oneshot(
                HttpRequest::get("/")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
        let body = body_text(second).await;
        assert!(body.contains("logged off after inactivity"));
        assert!(
            state.0.sessions.lock().unwrap().is_empty(),
            "the idle session is destroyed, not just refused"
        );
        // Even the refused attempt is audited (two lines: mint + refusal).
        assert_eq!(audit_lines(&state).len(), 2);
    }

    #[tokio::test]
    async fn photo_upload_stub_accepts_multipart_and_labels_the_encryption_todo() {
        let state = test_state(Duration::from_secs(900));
        let boundary = "X-SCAFFOLD-TEST";
        let payload = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"patient_id\"\r\n\r\npt-002\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"wound-day4.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nnot-really-a-jpeg\r\n\
             --{boundary}--\r\n"
        );
        let request = HttpRequest::post("/photos")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(payload))
            .unwrap();
        let response = app(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        assert!(
            body.contains("NOT encrypted at rest"),
            "the stub never claims the control: {body}"
        );

        let photos = state.0.photos.lock().unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].patient_id, "pt-002");
        assert_eq!(photos[0].filename, "wound-day4.jpg");
        assert!(!photos[0].encrypted_at_rest);
    }
}
