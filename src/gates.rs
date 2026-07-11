//! Gate engine: the promotion checklist as code — the product.
//!
//! Gates are plugins behind one small trait, the way Vault mounts secret
//! engines and Nomad mounts task drivers. The platform ships a built-in set;
//! a hospital's own gates (IRB review, model risk) register alongside them.
//! The engine evaluates; it never deploys and never edits the app.

use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::state::{AppRecord, DataSource};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum GateStatus {
    Pass,
    Fail { reason: String, fixable: bool },
}

#[derive(Clone, Debug, Serialize)]
pub struct GateResult {
    pub id: String,
    pub title: String,
    #[serde(flatten)]
    pub outcome: GateStatus,
}

#[derive(Clone, Debug, Serialize)]
pub struct GateReport {
    pub app_id: String,
    pub app_version: u32,
    pub results: Vec<GateResult>,
    pub passed: usize,
    pub total: usize,
    pub green: bool,
}

impl GateReport {
    pub fn summary(&self) -> String {
        format!("{}/{}", self.passed, self.total)
    }

    pub fn failing(&self) -> Vec<&GateResult> {
        self.results
            .iter()
            .filter(|r| r.outcome != GateStatus::Pass)
            .collect()
    }
}

/// One compliance check. Implementations must be pure over the app record:
/// same app in, same verdict out, so a gate report is reproducible evidence.
pub trait Gate: Send + Sync {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn evaluate(&self, app: &AppRecord) -> GateStatus;
}

/// A gate satisfied by a wired control on the app record. Most HIPAA
/// technical safeguards reduce to this shape.
///
/// TODO(#3): demo semantics — controls are self-reported by the scaffold and
/// agent, so these verdicts are claims, not evidence. Real gates derive from
/// artifacts: static analysis of generated source, observed sandbox egress,
/// real dependency scans. The trait also grows Packer's validate/execute
/// split so the full gate plan can dry-run during preview.
struct ControlGate {
    id: &'static str,
    title: &'static str,
    missing: &'static str,
    fixable: bool,
}

impl Gate for ControlGate {
    fn id(&self) -> &'static str {
        self.id
    }
    fn title(&self) -> &'static str {
        self.title
    }
    fn evaluate(&self, app: &AppRecord) -> GateStatus {
        if app.controls.contains(self.id) {
            GateStatus::Pass
        } else {
            GateStatus::Fail {
                reason: self.missing.to_string(),
                fixable: self.fixable,
            }
        }
    }
}

/// Third-party calls must resolve against the BAA'd allowlist. An un-BAA'd
/// AI endpoint is the single most common way a vibe-coded tool leaks PHI.
struct AiAllowlistGate;

const ENDPOINT_ALLOWLIST: &[&str] = &[
    "vault.internal",
    "postgres.internal",
    "api.anthropic.com", // platform LLM key, scoped per environment, under BAA
];

impl Gate for AiAllowlistGate {
    fn id(&self) -> &'static str {
        "ai-allowlist"
    }
    fn title(&self) -> &'static str {
        "no un-approved AI calls"
    }
    fn evaluate(&self, app: &AppRecord) -> GateStatus {
        let rogue: Vec<&str> = app
            .external_calls
            .iter()
            .map(String::as_str)
            .filter(|c| !ENDPOINT_ALLOWLIST.contains(c))
            .collect();
        if rogue.is_empty() {
            GateStatus::Pass
        } else {
            GateStatus::Fail {
                reason: format!(
                    "calls endpoints outside the BAA allowlist: {}",
                    rogue.join(", ")
                ),
                fixable: false,
            }
        }
    }
}

/// The sandbox must only ever have seen synthetic data. This is evaluated,
/// not assumed, so the gate report can attest to it.
struct SyntheticOnlyGate;

impl Gate for SyntheticOnlyGate {
    fn id(&self) -> &'static str {
        "synthetic-only"
    }
    fn title(&self) -> &'static str {
        "sandbox saw synthetic data only"
    }
    fn evaluate(&self, app: &AppRecord) -> GateStatus {
        match &app.data_source {
            DataSource::Synthetic(_) => GateStatus::Pass,
            DataSource::Tenant(db) => GateStatus::Fail {
                reason: format!("sandbox is wired to tenant data source {db}"),
                fixable: false,
            },
        }
    }
}

static REGISTRY: LazyLock<Vec<Box<dyn Gate>>> = LazyLock::new(|| {
    vec![
        Box::new(ControlGate {
            id: "phi-encryption",
            title: "encryption on all patient fields",
            missing: "one or more PHI fields lack hipaa-core field-level encryption",
            fixable: false,
        }),
        Box::new(ControlGate {
            id: "audit-log",
            title: "audit log on every data access",
            missing: "a data-touching route is missing hipaa-core audit middleware",
            fixable: false,
        }),
        Box::new(AiAllowlistGate),
        Box::new(ControlGate {
            id: "dependency-scan",
            title: "dependency scan clean",
            missing: "dependency scan has unresolved findings",
            fixable: false,
        }),
        Box::new(ControlGate {
            id: "auto-logoff",
            title: "auto-logoff after idle",
            missing: "auto-logoff after idle — not wired",
            fixable: true,
        }),
        Box::new(SyntheticOnlyGate),
        Box::new(ControlGate {
            id: "access-roles",
            title: "access roles for staff",
            missing: "staff-facing surface has no role-based access control",
            fixable: true,
        }),
        Box::new(ControlGate {
            id: "escalation-path",
            title: "clinical escalation path",
            missing: "no escalation path for out-of-range or urgent findings",
            fixable: true,
        }),
        Box::new(ControlGate {
            id: "human-review",
            title: "platform review attached",
            missing: "compliance review not yet run — request co-sign review",
            fixable: false,
        }),
    ]
});

fn gate(id: &str) -> Option<&'static dyn Gate> {
    REGISTRY.iter().find(|g| g.id() == id).map(|g| g.as_ref())
}

pub fn known_gate(id: &str) -> bool {
    gate(id).is_some()
}

pub fn gate_fixable(id: &str) -> bool {
    // Whether "fix it for me" may wire this control directly.
    matches!(id, "auto-logoff" | "access-roles" | "escalation-path")
}

/// Run the preflight: evaluate exactly the gates the app's pack requires,
/// in the pack's declared order.
pub fn preflight(app: &AppRecord, required: &[String]) -> GateReport {
    let results: Vec<GateResult> = required
        .iter()
        .map(|id| match gate(id) {
            Some(g) => GateResult {
                id: g.id().to_string(),
                title: g.title().to_string(),
                outcome: g.evaluate(app),
            },
            None => GateResult {
                id: id.clone(),
                title: format!("unknown gate {id}"),
                outcome: GateStatus::Fail {
                    reason: format!("pack requires gate {id:?} but no such gate is registered"),
                    fixable: false,
                },
            },
        })
        .collect();
    let passed = results
        .iter()
        .filter(|r| r.outcome == GateStatus::Pass)
        .count();
    let total = results.len();
    GateReport {
        app_id: app.id.clone(),
        app_version: app.current_version,
        green: passed == total && total > 0,
        passed,
        total,
        results,
    }
}

/// The platform reviewer's attestation note (storyboard 1c's co-sign card):
/// a plain-language verdict derived from the same report the modal shows.
pub fn reviewer_note(report: &GateReport, tier: u8) -> String {
    let audience = if tier >= 3 {
        "patient-facing"
    } else {
        "practice-facing"
    };
    if report.green {
        format!(
            "Meets release criteria for a {audience} tool ({} checks green). \
             Re-review required if messaging, new data fields, or external calls are added.",
            report.summary()
        )
    } else {
        let failing: Vec<String> = report.failing().iter().map(|r| r.title.clone()).collect();
        format!(
            "Not ready to release: {} of {} checks failing — {}.",
            report.total - report.passed,
            report.total,
            failing.join("; ")
        )
    }
}

/// Machine-readable gate map for the compliance meter (storyboard 1b).
pub fn meter(report: &GateReport) -> BTreeMap<String, bool> {
    report
        .results
        .iter()
        .map(|r| (r.id.clone(), r.outcome == GateStatus::Pass))
        .collect()
}
