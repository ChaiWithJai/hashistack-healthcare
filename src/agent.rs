//! Agent service: generates and iterates app scaffolds. It never deploys.
//!
//! The driver boundary mirrors a Nomad task driver: the control plane speaks
//! one small interface and the model behind it is swappable (rule-based for
//! Phase 0 tests and offline dev, Claude driver next) without any caller
//! noticing — workflows over technologies.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

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

/// OpenAI-compatible chat-completions driver: one client shape covers vLLM,
/// llama.cpp, and LM Studio (the "local" tier, investigation 0002 D1) and the
/// frontier endpoint stub. It never routes itself — the escalation ladder's
/// verifier decides whether its output is accepted, so a wrong, empty, or
/// unreachable model degrades to a rejected attempt, never a broken app.
///
/// The edit protocol is deliberately constrained (the pack shrinks the job
/// the model must do): the model replies with one JSON object —
/// `{"feature": str?, "controls": [gate ids], "drop_controls": [gate ids],
/// "message": str?}` for iterate, `{"steps": [str]}` for scaffold. Anything
/// unparseable becomes a no-op edit the verifier rejects as `empty-edit`.
pub struct HttpModelDriver {
    tier: &'static str,
    base_url: String,
}

impl HttpModelDriver {
    /// In-VPC model endpoint (`LOCAL_MODEL_URL`).
    pub fn local(base_url: String) -> Self {
        Self {
            tier: "local",
            base_url,
        }
    }

    /// Frontier endpoint stub (`FRONTIER_MODEL_URL`) — same client shape, so
    /// the real ClaudeDriver slots in behind the same rung. Phase 0 never
    /// calls a real API: the URL is only ever a test double or in-VPC proxy.
    pub fn frontier(base_url: String) -> Self {
        Self {
            tier: "frontier",
            base_url,
        }
    }

    fn no_op(&self, why: String) -> AgentReply {
        AgentReply {
            message: why,
            added_feature: None,
            wired_controls: Vec::new(),
            compliance_nudge: None,
        }
    }
}

/// The constrained edit a model may propose. `drop_controls` exists so a bad
/// edit that loses a safeguard is representable — and therefore catchable by
/// the verifier's gate-regression check.
#[derive(Debug, Default, Deserialize)]
struct EditSpec {
    #[serde(default)]
    feature: Option<String>,
    #[serde(default)]
    controls: Vec<String>,
    #[serde(default)]
    drop_controls: Vec<String>,
    #[serde(default)]
    message: Option<String>,
}

impl AgentDriver for HttpModelDriver {
    fn scaffold(&self, pack: &PackManifest, prompt: &str) -> Vec<ScaffoldStep> {
        let body = json!({
            "model": self.tier,
            "messages": [
                {"role": "system", "content":
                    "You scaffold a HIPAA-scaffolded clinical app from a pack. \
                     Reply with exactly one JSON object: {\"steps\": [string]}."},
                {"role": "user", "content": format!("pack {}: {}", pack.id, prompt)},
            ],
        });
        let Ok(content) = post_chat(&self.base_url, &body) else {
            return Vec::new(); // verifier rejects an empty scaffold
        };
        #[derive(Deserialize)]
        struct StepSpec {
            #[serde(default)]
            steps: Vec<String>,
        }
        let spec: StepSpec = serde_json::from_str(&content).unwrap_or(StepSpec { steps: vec![] });
        spec.steps
            .into_iter()
            .map(|label| ScaffoldStep { label, done: true })
            .collect()
    }

    fn iterate(
        &self,
        app: &mut AppRecord,
        instruction: &str,
        required_gates: &[String],
    ) -> AgentReply {
        let body = json!({
            "model": self.tier,
            "messages": [
                {"role": "system", "content": format!(
                    "You apply one constrained edit to a scaffolded clinical app. \
                     Reply with exactly one JSON object: {{\"feature\": string, \
                     \"controls\": [gate ids to wire], \"drop_controls\": [], \
                     \"message\": string}}. Required gates: {}.",
                    required_gates.join(", ")
                )},
                {"role": "user", "content": instruction},
            ],
        });
        let content = match post_chat(&self.base_url, &body) {
            Ok(c) => c,
            Err(e) => return self.no_op(format!("{} tier unreachable: {e}", self.tier)),
        };
        let edit: EditSpec = match serde_json::from_str(&content) {
            Ok(e) => e,
            Err(_) => {
                return self.no_op(format!("{} tier returned an unparseable edit", self.tier))
            }
        };

        let mut wired = Vec::new();
        for control in &edit.controls {
            if app.controls.insert(control.clone()) {
                wired.push(control.clone());
            }
        }
        for control in &edit.drop_controls {
            app.controls.remove(control);
        }
        if let Some(feature) = &edit.feature {
            app.features.push(feature.clone());
            app.routes += 1;
        }

        let message = edit.message.unwrap_or_else(|| {
            format!(
                "✓ done — {}. Nothing leaves the sandbox yet.",
                edit.feature.as_deref().unwrap_or("edit applied")
            )
        });
        AgentReply {
            message,
            added_feature: edit.feature,
            wired_controls: wired,
            compliance_nudge: None,
        }
    }
}

/// Minimal OpenAI-compatible POST /v1/chat/completions over std TcpStream —
/// no HTTP client dependency for an endpoint that is loopback/in-VPC by
/// definition (the ai-allowlist gate is the topology, not a library).
/// Returns choices[0].message.content.
fn post_chat(base_url: &str, body: &serde_json::Value) -> Result<String> {
    let hostport = base_url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow!("model endpoint must be http:// (in-VPC): {base_url}"))?;
    let hostport = hostport.split('/').next().unwrap_or(hostport);

    let payload = body.to_string();
    let stream = TcpStream::connect(hostport)
        .with_context(|| format!("connecting to model endpoint {hostport}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut stream = stream;
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nhost: {hostport}\r\n\
         content-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(request.as_bytes())?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let (head, response_body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed HTTP response from model endpoint"))?;
    let status_line = head.lines().next().unwrap_or("");
    if !status_line.contains(" 200 ") && !status_line.ends_with(" 200") {
        bail!("model endpoint returned {status_line:?}");
    }
    let value: serde_json::Value =
        serde_json::from_str(response_body.trim()).context("model response is not JSON")?;
    value["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("model response has no choices[0].message.content"))
}

fn summarize_feature(instruction: &str) -> String {
    let trimmed = instruction.trim().trim_matches('"');
    let mut summary: String = trimmed.chars().take(72).collect();
    if trimmed.chars().count() > 72 {
        summary.push('…');
    }
    summary
}
