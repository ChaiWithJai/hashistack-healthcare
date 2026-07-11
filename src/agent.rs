//! Agent service: generates and iterates app scaffolds. It never deploys.
//!
//! The driver boundary mirrors a Nomad task driver: the control plane speaks
//! one small interface and the model behind it is swappable (rule-based for
//! Phase 0 tests and offline dev, Claude driver next) without any caller
//! noticing — workflows over technologies.
//!
//! Treatment 4a (#4, investigation 0002 D1): [`RouterDriver`] is a composite
//! driver — routing policy hardcoded in this module — that sends scaffold
//! authorship to the frontier model and constrained iterate edits to an
//! in-VPC local model, escalating automatically when a local edit fails
//! validation. The doctor never picks a model; every decision is drained
//! into the audit stream by the API layer.

use serde::{Deserialize, Serialize};
use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

use crate::gates;
use crate::packs::PackManifest;
use crate::state::AppRecord;

#[derive(Clone, Debug, Serialize)]
pub struct ScaffoldStep {
    pub label: String,
    pub done: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentReply {
    pub message: String,
    pub added_feature: Option<String>,
    pub wired_controls: Vec<String>,
    /// A heads-up when the edit touches an open compliance check
    /// (storyboard 1b③: "a staff queue needs access roles").
    pub compliance_nudge: Option<String>,
}

pub trait AgentDriver: Send + Sync {
    /// Build the initial scaffold from the pack. Returns the step list the
    /// doctor watches tick by during generate.
    fn scaffold(&self, pack: &PackManifest, prompt: &str) -> Vec<ScaffoldStep>;

    /// Apply one conversational edit to the app record.
    fn iterate(
        &self,
        app: &mut AppRecord,
        instruction: &str,
        required_gates: &[String],
    ) -> AgentReply;
}

/// Deterministic Phase 0 driver: keyword rules instead of a model, so the
/// full describe→audit loop runs offline and in CI with stable assertions.
///
/// TODO(#4): the ClaudeDriver lands behind this same trait — scaffold()
/// renders the pack template into a real workspace, iterate() produces
/// checkpointed diffs, prompts versioned in packs/<id>/prompts/. This
/// rule-based driver stays as the offline/CI driver to prove the swap.
pub struct RuleBasedDriver;

impl AgentDriver for RuleBasedDriver {
    fn scaffold(&self, pack: &PackManifest, _prompt: &str) -> Vec<ScaffoldStep> {
        let mut steps = vec![ScaffoldStep {
            label: "scaffolding from pack…".to_string(),
            done: true,
        }];
        steps.extend(pack.scaffold.iter().map(|f| ScaffoldStep {
            label: f.clone(),
            done: true,
        }));
        steps
    }

    fn iterate(
        &self,
        app: &mut AppRecord,
        instruction: &str,
        required_gates: &[String],
    ) -> AgentReply {
        let lower = instruction.to_lowercase();
        let mut wired = Vec::new();

        let wire = |app: &mut AppRecord, control: &str, wired: &mut Vec<String>| {
            if app.controls.insert(control.to_string()) {
                wired.push(control.to_string());
            }
        };

        if lower.contains("role") {
            wire(app, "access-roles", &mut wired);
        }
        if lower.contains("logoff") || lower.contains("log off") || lower.contains("idle") {
            wire(app, "auto-logoff", &mut wired);
        }
        if lower.contains("escalat") || lower.contains("flag") {
            wire(app, "escalation-path", &mut wired);
        }

        let feature = summarize_feature(instruction);
        app.features.push(feature.clone());
        app.routes += 1;
        // Every generated route arrives with hipaa-core middleware attached;
        // that invariant is what the audit-log gate re-checks at preflight.

        let compliance_nudge = if lower.contains("queue") || lower.contains("staff") {
            let roles_required = required_gates.iter().any(|g| g == "access-roles");
            let roles_missing = !app.controls.contains("access-roles");
            (roles_required && roles_missing).then(|| {
                "a staff queue needs access roles — that's one of your open checks. \
                 Want me to wire roles now?"
                    .to_string()
            })
        } else {
            None
        };

        let message = if wired.is_empty() {
            format!("✓ done — {feature}. Nothing leaves the sandbox yet.")
        } else {
            format!("✓ done — {feature}. Also wired: {}.", wired.join(", "))
        };

        AgentReply {
            message,
            added_feature: Some(feature),
            wired_controls: wired,
            compliance_nudge,
        }
    }
}

fn summarize_feature(instruction: &str) -> String {
    let trimmed = instruction.trim().trim_matches('"');
    let mut summary: String = trimmed.chars().take(72).collect();
    if trimmed.chars().count() > 72 {
        summary.push('…');
    }
    summary
}

// ---------- treatment 4a: router-in-driver ----------

const LOCAL_MODEL: &str = "qwen3-coder";
const FRONTIER_MODEL: &str = "claude-frontier";

/// One routing decision made by the [`RouterDriver`]. The API layer drains
/// these after each driver call and records them as `agent.routed` audit
/// events — routing is only trustworthy if it is observable.
#[derive(Clone, Debug, Serialize)]
pub struct RoutingDecision {
    /// What was being routed: `"scaffold"` or `"iterate v3"`.
    pub task: String,
    /// The route first attempted, e.g. `"local (qwen3-coder)"`.
    pub route: String,
    /// `"ok"`, or the failure + escalation story.
    pub outcome: String,
}

impl RoutingDecision {
    pub fn detail(&self) -> String {
        format!("{} → {} {}", self.task, self.route, self.outcome)
    }
}

/// Minimal OpenAI-compatible chat-completions client over plain HTTP/1.1,
/// std-only on purpose: the endpoint is in-VPC (vLLM, llama.cpp, and
/// LM Studio all speak this shape — investigation 0002, "no model lock-in"),
/// and the treatment adds zero dependencies. Blocking is acceptable at
/// Phase 0: the API layer already serializes on the platform lock.
struct ChatClient {
    base_url: String,
    model: &'static str,
}

impl ChatClient {
    fn new(base_url: String, model: &'static str) -> Self {
        Self { base_url, model }
    }

    fn chat(&self, system: &str, user: &str) -> Result<String, String> {
        let rest = self
            .base_url
            .strip_prefix("http://")
            .ok_or_else(|| "config (only in-VPC http:// endpoints are supported)".to_string())?;
        let (host, prefix) = match rest.split_once('/') {
            Some((h, p)) => (h, format!("/{}", p.trim_end_matches('/'))),
            None => (rest, String::new()),
        };
        let path = format!("{prefix}/v1/chat/completions");
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "temperature": 0,
        })
        .to_string();

        let addr = host
            .to_socket_addrs()
            .map_err(|e| format!("transport (resolve {host}: {e})"))?
            .next()
            .ok_or_else(|| format!("transport (no address for {host})"))?;
        let stream_err = |e: std::io::Error| format!("transport ({e})");
        let mut stream =
            TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(stream_err)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(5))))
            .map_err(stream_err)?;

        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).map_err(stream_err)?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(stream_err)?;
        let text = String::from_utf8_lossy(&raw);
        let (head, response_body) = text
            .split_once("\r\n\r\n")
            .ok_or_else(|| "transport (malformed http response)".to_string())?;
        let status = head.lines().next().unwrap_or_default().trim();
        if !status.contains(" 200") {
            return Err(format!("bad-status ({status})"));
        }
        let envelope: serde_json::Value = serde_json::from_str(response_body.trim())
            .map_err(|_| "malformed-envelope (body is not JSON)".to_string())?;
        envelope["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "malformed-envelope (no choices[0].message.content)".to_string())
    }
}

/// The constrained edit protocol both model drivers speak: the pack system
/// shrinks the job the model must do (investigation 0002) — the model never
/// free-writes state, it proposes one well-shaped slot edit that the router
/// validates against the gate engine before committing.
const EDIT_SYSTEM_PROMPT: &str =
    "You apply one scaffold-constrained edit to a clinical app record. Respond with exactly one \
     JSON object and nothing else: {\"summary\":\"one line describing the edit\",\
     \"wire_controls\":[\"gate-id\"],\"drop_controls\":[]}. Gate ids come from the pack's \
     required gates. No prose, no code fences.";

const SCAFFOLD_SYSTEM_PROMPT: &str =
    "You author the initial scaffold for a clinical app from its signed pack template. \
     Confirm the pack's scaffold plan in one line.";

#[derive(Deserialize)]
struct ProposedEdit {
    summary: String,
    #[serde(default)]
    wire_controls: Vec<String>,
    #[serde(default)]
    drop_controls: Vec<String>,
}

fn request_edit(
    client: &ChatClient,
    app: &AppRecord,
    instruction: &str,
    required_gates: &[String],
) -> Result<ProposedEdit, String> {
    let user = format!(
        "app {} v{} — required gates: [{}] — wired controls: [{}]\ninstruction: {instruction}",
        app.id,
        app.current_version,
        required_gates.join(", "),
        app.controls.iter().cloned().collect::<Vec<_>>().join(", "),
    );
    let content = client.chat(EDIT_SYSTEM_PROMPT, &user)?;
    let edit: ProposedEdit = serde_json::from_str(content.trim())
        .map_err(|_| "invalid-reply (not a well-formed edit object)".to_string())?;
    if edit.summary.trim().is_empty() {
        return Err("invalid-reply (empty summary)".to_string());
    }
    Ok(edit)
}

/// Apply a validated model edit to the record — the same structural shape a
/// rule-based edit takes, so gates and restore work identically either way.
fn apply_edit(app: &mut AppRecord, edit: &ProposedEdit) -> AgentReply {
    let mut wired = Vec::new();
    for control in &edit.wire_controls {
        if app.controls.insert(control.clone()) {
            wired.push(control.clone());
        }
    }
    for control in &edit.drop_controls {
        app.controls.remove(control);
    }
    let feature = summarize_feature(&edit.summary);
    app.features.push(feature.clone());
    app.routes += 1;
    let message = if wired.is_empty() {
        format!("✓ done — {feature}. Nothing leaves the sandbox yet.")
    } else {
        format!("✓ done — {feature}. Also wired: {}.", wired.join(", "))
    };
    AgentReply {
        message,
        added_feature: Some(feature),
        wired_controls: wired,
        compliance_nudge: None,
    }
}

/// In-VPC open-weight coder behind any OpenAI-compatible endpoint
/// (`LOCAL_MODEL_URL` → vLLM / llama.cpp / LM Studio, interchangeable).
/// It only ever *proposes* edits; the router validates and commits.
pub struct LocalDriver {
    client: ChatClient,
}

impl LocalDriver {
    pub fn new(base_url: String) -> Self {
        Self {
            client: ChatClient::new(base_url, LOCAL_MODEL),
        }
    }

    fn route_label(&self) -> String {
        format!("local ({})", self.client.model)
    }

    fn propose(
        &self,
        app: &mut AppRecord,
        instruction: &str,
        required_gates: &[String],
    ) -> Result<AgentReply, String> {
        let edit = request_edit(&self.client, app, instruction, required_gates)?;
        Ok(apply_edit(app, &edit))
    }
}

/// Stub for the Claude driver: the same HTTP client shape pointed at
/// `FRONTIER_MODEL_URL`. It never calls a real API — with no URL configured
/// it degrades to the deterministic rule-based edit, and any HTTP failure
/// also falls back to rules so the doctor's edit always lands.
pub struct FrontierDriver {
    client: Option<ChatClient>,
}

impl FrontierDriver {
    pub fn new(base_url: Option<String>) -> Self {
        Self {
            client: base_url.map(|url| ChatClient::new(url, FRONTIER_MODEL)),
        }
    }

    fn route_label(&self) -> String {
        match &self.client {
            Some(c) => format!("frontier ({})", c.model),
            None => format!("frontier ({FRONTIER_MODEL} stub, offline)"),
        }
    }

    /// Returns the reply plus an outcome string for the audit trail.
    fn iterate_with_outcome(
        &self,
        app: &mut AppRecord,
        instruction: &str,
        required_gates: &[String],
    ) -> (AgentReply, String) {
        if let Some(client) = &self.client {
            let mut candidate = app.clone();
            match request_edit(client, &candidate, instruction, required_gates)
                .map(|edit| apply_edit(&mut candidate, &edit))
            {
                Ok(reply) => {
                    *app = candidate;
                    return (reply, "ok".to_string());
                }
                Err(why) => {
                    let reply = RuleBasedDriver.iterate(app, instruction, required_gates);
                    return (reply, format!("{why} → rule-based fallback"));
                }
            }
        }
        let reply = RuleBasedDriver.iterate(app, instruction, required_gates);
        (reply, "ok".to_string())
    }

    fn scaffold_outcome(&self, pack: &PackManifest, prompt: &str) -> String {
        match &self.client {
            Some(client) => {
                let user = format!("pack {} ({} steps): {prompt}", pack.id, pack.scaffold.len());
                match client.chat(SCAFFOLD_SYSTEM_PROMPT, &user) {
                    Ok(_) => "ok".to_string(),
                    Err(why) => format!("{why} → pack template fallback"),
                }
            }
            None => "ok".to_string(),
        }
    }
}

/// Composite driver with the routing policy hardcoded here, in code:
///
/// - **scaffold → frontier**, always: first generation is the product's
///   first impression (investigation 0002, "where we must NOT simplify").
/// - **iterate → local first**: the constrained, scaffold-shaped edit is the
///   job a mid-size open-weight coder can clear. The candidate edit lands on
///   a *cloned* record; if the reply is invalid or `gates::preflight` on the
///   clone regresses versus before the edit, the clone is discarded and the
///   untouched record is escalated to the frontier driver — automatically
///   and invisibly. The doctor never picks a model (Tao 1).
/// - **no env configured → passthrough**: exactly today's [`RuleBasedDriver`]
///   behavior, no routing events, so CI and offline dev are unchanged.
pub struct RouterDriver {
    local: Option<LocalDriver>,
    frontier: FrontierDriver,
    active: bool,
    decisions: Mutex<Vec<RoutingDecision>>,
}

impl RouterDriver {
    /// Reads `LOCAL_MODEL_URL` and `FRONTIER_MODEL_URL`. Neither set →
    /// rule-based passthrough, today's exact behavior.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var("LOCAL_MODEL_URL").ok(),
            std::env::var("FRONTIER_MODEL_URL").ok(),
        )
    }

    pub fn new(local_url: Option<String>, frontier_url: Option<String>) -> Self {
        let active = local_url.is_some() || frontier_url.is_some();
        Self {
            local: local_url.map(LocalDriver::new),
            frontier: FrontierDriver::new(frontier_url),
            active,
            decisions: Mutex::new(Vec::new()),
        }
    }

    pub fn is_passthrough(&self) -> bool {
        !self.active
    }

    /// Hand pending routing decisions to the caller (the API layer), which
    /// records them in the audit stream. Every decision must reach audit —
    /// the driver holds them only between the call and the drain.
    pub fn drain_decisions(&self) -> Vec<RoutingDecision> {
        std::mem::take(&mut *self.decisions.lock().unwrap())
    }

    fn push(&self, task: &str, route: &str, outcome: &str) {
        self.decisions.lock().unwrap().push(RoutingDecision {
            task: task.to_string(),
            route: route.to_string(),
            outcome: outcome.to_string(),
        });
    }
}

impl AgentDriver for RouterDriver {
    fn scaffold(&self, pack: &PackManifest, prompt: &str) -> Vec<ScaffoldStep> {
        if self.is_passthrough() {
            return RuleBasedDriver.scaffold(pack, prompt);
        }
        // Policy: scaffold authorship always routes frontier.
        let outcome = self.frontier.scaffold_outcome(pack, prompt);
        self.push("scaffold", &self.frontier.route_label(), &outcome);
        RuleBasedDriver.scaffold(pack, prompt)
    }

    fn iterate(
        &self,
        app: &mut AppRecord,
        instruction: &str,
        required_gates: &[String],
    ) -> AgentReply {
        if self.is_passthrough() {
            return RuleBasedDriver.iterate(app, instruction, required_gates);
        }
        let task = format!("iterate v{}", app.current_version + 1);

        if let Some(local) = &self.local {
            // Policy: iterate routes local first, validated before commit.
            let before = gates::preflight(app, required_gates);
            let mut candidate = app.clone();
            let why = match local.propose(&mut candidate, instruction, required_gates) {
                Ok(reply) => {
                    let after = gates::preflight(&candidate, required_gates);
                    if after.passed >= before.passed {
                        *app = candidate;
                        self.push(&task, &local.route_label(), "ok");
                        return reply;
                    }
                    format!(
                        "gate-regression ({} → {})",
                        before.summary(),
                        after.summary()
                    )
                }
                Err(why) => why,
            };
            // The regressive candidate is discarded; the frontier driver
            // starts from the pristine record.
            let (reply, outcome) =
                self.frontier
                    .iterate_with_outcome(app, instruction, required_gates);
            self.push(
                &task,
                &local.route_label(),
                &format!(
                    "failed {why} → escalated {} {outcome}",
                    self.frontier.route_label()
                ),
            );
            return reply;
        }

        // No local endpoint configured: iterate routes straight to frontier.
        let (reply, outcome) = self
            .frontier
            .iterate_with_outcome(app, instruction, required_gates);
        self.push(&task, &self.frontier.route_label(), &outcome);
        reply
    }
}
