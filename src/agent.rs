//! Agent service: generates and iterates app scaffolds. It never deploys.
//!
//! The driver boundary mirrors a Nomad task driver: the control plane speaks
//! one small interface and the model behind it is swappable (rule-based for
//! Phase 0 tests and offline dev, Claude driver next) without any caller
//! noticing — workflows over technologies.

use serde::Serialize;

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
