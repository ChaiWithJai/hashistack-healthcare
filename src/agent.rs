//! Agent service: generates and iterates app scaffolds. It never deploys.
//!
//! The driver boundary mirrors a Nomad task driver: the control plane speaks
//! one small interface and the model behind it is swappable (rule-based for
//! Phase 0 tests and offline dev, Claude driver next) without any caller
//! noticing — workflows over technologies.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::packs::{EscalationReason, PackManifest, RoutingPolicy, RoutingTier};
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

// ---------- treatment 4b: pack-declared routing ----------
//
// The dispatcher below is deliberately dumb: it holds no routing opinions of
// its own. The pack's signed `routing` policy names a tier per operation and
// the failures that earn a frontier escalation; the dispatcher just reads the
// policy, calls the named tier, and reports every decision for the audit
// stream. Codification over code — the same reason gates live in pack.hcl.

/// One routing decision, ready for the audit log. `detail` always cites the
/// policy that produced the decision (pack override or platform default).
#[derive(Clone, Debug)]
pub struct RoutingDecision {
    /// Audit action: `agent.routed` or `agent.escalated`.
    pub action: &'static str,
    pub detail: String,
}

/// The edit shape every model tier must return (as the JSON content of an
/// OpenAI-compatible chat completion). Anything else is an invalid-edit.
#[derive(Debug, Deserialize)]
struct ModelEdit {
    message: String,
    #[serde(default)]
    added_feature: Option<String>,
    #[serde(default)]
    wired_controls: Vec<String>,
    #[serde(default)]
    removed_controls: Vec<String>,
}

/// Minimal OpenAI-compatible chat client over plain HTTP/1.1 — enough for an
/// in-VPC endpoint (vLLM, llama.cpp, LM Studio) without pulling in an HTTP
/// stack. Deliberately unsupported: TLS, chunked responses, streaming. The
/// real frontier client needs all three and is out of scope for Phase 0.
struct OpenAiCompatClient {
    base: String,
}

impl OpenAiCompatClient {
    fn new(base: impl Into<String>) -> Self {
        Self { base: base.into() }
    }

    fn try_edit(&self, instruction: &str, required_gates: &[String]) -> Result<ModelEdit, String> {
        let system = format!(
            "You apply one conversational edit to a pack-scaffolded healthcare app. \
             Reply with ONLY a JSON object: {{\"message\": string, \"added_feature\": \
             string|null, \"wired_controls\": [string], \"removed_controls\": [string]}}. \
             Compliance gates in force: {}.",
            required_gates.join(", ")
        );
        let content = self.chat(&system, instruction)?;
        serde_json::from_str(&content)
            .map_err(|e| format!("model reply is not a well-formed edit: {e}"))
    }

    fn chat(&self, system: &str, user: &str) -> Result<String, String> {
        let body = serde_json::json!({
            "model": "pack-routed",
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        })
        .to_string();
        let response = self.post_json("/v1/chat/completions", &body)?;
        let parsed: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| format!("endpoint returned non-JSON: {e}"))?;
        parsed["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "endpoint reply carries no choices[0].message.content".to_string())
    }

    fn post_json(&self, path: &str, body: &str) -> Result<String, String> {
        let rest = self
            .base
            .strip_prefix("http://")
            .ok_or_else(|| {
                format!(
                    "unsupported endpoint {:?} — only http:// (in-VPC)",
                    self.base
                )
            })?
            .trim_end_matches('/');
        let (host, prefix) = match rest.split_once('/') {
            Some((h, p)) => (h, format!("/{p}")),
            None => (rest, String::new()),
        };
        let mut stream = TcpStream::connect(host).map_err(|e| format!("connect {host}: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(10))))
            .map_err(|e| format!("socket timeouts: {e}"))?;
        let request = format!(
            "POST {prefix}{path} HTTP/1.1\r\nhost: {host}\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("send: {e}"))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| format!("recv: {e}"))?;
        let text = String::from_utf8_lossy(&raw);
        let (head, response_body) = text
            .split_once("\r\n\r\n")
            .ok_or_else(|| "malformed HTTP response".to_string())?;
        let status_line = head.lines().next().unwrap_or_default();
        if !status_line.contains(" 200") {
            return Err(format!("endpoint returned {status_line:?}"));
        }
        Ok(response_body.to_string())
    }
}

/// In-VPC open-weight coder behind an OpenAI-compatible endpoint
/// (`LOCAL_MODEL_URL`): vLLM, llama.cpp, and LM Studio interchangeably.
pub struct LocalDriver {
    client: OpenAiCompatClient,
}

impl LocalDriver {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            client: OpenAiCompatClient::new(endpoint),
        }
    }
}

/// Frontier stub: the exact same client shape pointed at
/// `FRONTIER_MODEL_URL`. It exists to prove the escalation path end-to-end;
/// it never calls a real API in this treatment (the real ClaudeDriver
/// replaces it behind the same tier name).
pub struct FrontierDriver {
    client: OpenAiCompatClient,
}

impl FrontierDriver {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            client: OpenAiCompatClient::new(endpoint),
        }
    }
}

/// Thin dispatcher: resolves each pack-named tier to a driver and records
/// why. A tier whose endpoint is unconfigured resolves to the deterministic
/// rules floor — with no env vars set, every operation lands exactly where
/// it did before this treatment existed.
pub struct Dispatcher {
    rules: RuleBasedDriver,
    local: Option<LocalDriver>,
    frontier: Option<FrontierDriver>,
}

impl Dispatcher {
    pub fn new(local_url: Option<String>, frontier_url: Option<String>) -> Self {
        Self {
            rules: RuleBasedDriver,
            local: local_url.map(LocalDriver::new),
            frontier: frontier_url.map(FrontierDriver::new),
        }
    }

    /// Production wiring: tiers come from the environment, policy from packs.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self::new(var("LOCAL_MODEL_URL"), var("FRONTIER_MODEL_URL"))
    }

    fn model(&self, tier: RoutingTier) -> Option<&OpenAiCompatClient> {
        match tier {
            RoutingTier::Rules => None,
            RoutingTier::Local => self.local.as_ref().map(|d| &d.client),
            RoutingTier::Frontier => self.frontier.as_ref().map(|d| &d.client),
        }
    }

    fn unresolved_note(tier: RoutingTier) -> &'static str {
        match tier {
            RoutingTier::Rules => "",
            RoutingTier::Local => " (LOCAL_MODEL_URL unset — resolved to rules)",
            RoutingTier::Frontier => " (FRONTIER_MODEL_URL unset — resolved to rules)",
        }
    }

    /// Scaffold per policy. Phase 0 renders every scaffold from the pack
    /// template regardless of tier — the pack constrains the model, which is
    /// the whole point — but the routing decision is still recorded so the
    /// audit stream shows who *would* have authored it.
    pub fn scaffold(
        &self,
        pack: &PackManifest,
        prompt: &str,
    ) -> (Vec<ScaffoldStep>, RoutingDecision) {
        let policy = pack.routing_policy();
        let steps = self.rules.scaffold(pack, prompt);
        let decision = RoutingDecision {
            action: "agent.routed",
            detail: format!(
                "per {}: scaffold→{} (phase 0: rendered from pack template)",
                pack.routing_source(),
                policy.scaffold
            ),
        };
        (steps, decision)
    }

    /// Review routing is policy-recorded the same way; the deterministic
    /// reviewer note in `gates` stands in for the frontier reviewer.
    pub fn route_review(&self, pack: &PackManifest) -> RoutingDecision {
        RoutingDecision {
            action: "agent.routed",
            detail: format!(
                "per {}: review→{} (phase 0: deterministic reviewer note)",
                pack.routing_source(),
                pack.routing_policy().review
            ),
        }
    }

    /// Apply one edit via the pack's iterate tier, escalating only for the
    /// failure classes the pack names in `escalate_on`.
    pub fn iterate(
        &self,
        app: &mut AppRecord,
        instruction: &str,
        pack: &PackManifest,
    ) -> (AgentReply, Vec<RoutingDecision>) {
        let policy = pack.routing_policy();
        let source = pack.routing_source();
        let tier = policy.iterate;
        let mut decisions = Vec::new();

        let Some(client) = self.model(tier) else {
            decisions.push(RoutingDecision {
                action: "agent.routed",
                detail: format!(
                    "per {source}: iterate→{tier}{}",
                    Self::unresolved_note(tier)
                ),
            });
            let reply = self.rules.iterate(app, instruction, &pack.gates);
            return (reply, decisions);
        };
        decisions.push(RoutingDecision {
            action: "agent.routed",
            detail: format!("per {source}: iterate→{tier}"),
        });

        match client.try_edit(instruction, &pack.gates) {
            Ok(edit) => {
                if let Some(gate) = regressed_gate(app, &pack.gates, &edit) {
                    let why = format!("edit would unwire satisfied gate {gate}");
                    if policy
                        .escalate_on
                        .contains(&EscalationReason::GateRegression)
                    {
                        return self.escalate(
                            app,
                            instruction,
                            pack,
                            &policy,
                            &source,
                            EscalationReason::GateRegression,
                            &why,
                            decisions,
                        );
                    }
                    // The pack declined the guard; the preflight gate engine
                    // remains the backstop before anything reaches prod.
                }
                (apply_edit(app, edit), decisions)
            }
            Err(why) => {
                if policy.escalate_on.contains(&EscalationReason::InvalidEdit) {
                    self.escalate(
                        app,
                        instruction,
                        pack,
                        &policy,
                        &source,
                        EscalationReason::InvalidEdit,
                        &why,
                        decisions,
                    )
                } else {
                    decisions.push(RoutingDecision {
                        action: "agent.routed",
                        detail: format!(
                            "per {source}: invalid-edit ({why}) not in escalate_on — \
                             deterministic fallback to rules"
                        ),
                    });
                    let reply = self.rules.iterate(app, instruction, &pack.gates);
                    (reply, decisions)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn escalate(
        &self,
        app: &mut AppRecord,
        instruction: &str,
        pack: &PackManifest,
        policy: &RoutingPolicy,
        source: &str,
        reason: EscalationReason,
        why: &str,
        mut decisions: Vec<RoutingDecision>,
    ) -> (AgentReply, Vec<RoutingDecision>) {
        let from = policy.iterate;
        match self
            .model(RoutingTier::Frontier)
            .filter(|_| from != RoutingTier::Frontier)
        {
            Some(frontier) => {
                decisions.push(RoutingDecision {
                    action: "agent.escalated",
                    detail: format!(
                        "per {source}: escalate_on {reason} — {why}; iterate {from}→frontier"
                    ),
                });
                match frontier.try_edit(instruction, &pack.gates) {
                    Ok(edit) => (apply_edit(app, edit), decisions),
                    Err(why2) => {
                        decisions.push(RoutingDecision {
                            action: "agent.routed",
                            detail: format!(
                                "per {source}: frontier escalation failed ({why2}) — \
                                 deterministic fallback to rules"
                            ),
                        });
                        (self.rules.iterate(app, instruction, &pack.gates), decisions)
                    }
                }
            }
            None => {
                decisions.push(RoutingDecision {
                    action: "agent.escalated",
                    detail: format!(
                        "per {source}: escalate_on {reason} — {why}; iterate {from}→frontier{}",
                        Self::unresolved_note(RoutingTier::Frontier)
                    ),
                });
                (self.rules.iterate(app, instruction, &pack.gates), decisions)
            }
        }
    }
}

/// Does the edit unwire a required gate the app currently satisfies?
fn regressed_gate(app: &AppRecord, required_gates: &[String], edit: &ModelEdit) -> Option<String> {
    edit.removed_controls
        .iter()
        .find(|c| required_gates.contains(c) && app.controls.contains(*c))
        .cloned()
}

/// Apply a well-formed model edit to the record, mirroring what the
/// rule-based driver does by hand.
fn apply_edit(app: &mut AppRecord, edit: ModelEdit) -> AgentReply {
    let mut wired = Vec::new();
    for control in edit.wired_controls {
        if app.controls.insert(control.clone()) {
            wired.push(control);
        }
    }
    for control in &edit.removed_controls {
        app.controls.remove(control);
    }
    if let Some(feature) = &edit.added_feature {
        app.features.push(feature.clone());
        app.routes += 1;
    }
    AgentReply {
        message: edit.message,
        added_feature: edit.added_feature,
        wired_controls: wired,
        compliance_nudge: None,
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
