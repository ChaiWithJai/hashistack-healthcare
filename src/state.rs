//! Control-plane state: the domain model behind every API client.
//!
//! One record type per lifecycle noun. Services (agent, gates, deploy, audit)
//! each own one verb over this state and nothing else.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::audit::AuditLog;
use crate::ladder::EscalationLadder;
use crate::packs::PackManifest;

pub type SharedPlatform = Arc<RwLock<Platform>>;

/// Where an app runs. Sandbox has no route to tenant databases; prod is the
/// only pool with tenant Postgres access. The gate sits between the two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Sandbox,
    Live,
}

/// What data the app can see. Synthetic in the sandbox, always.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "lowercase")]
pub enum DataSource {
    Synthetic(String),
    Tenant(String),
}

/// One conversational edit, logged like a chart addendum (storyboard 1c).
/// Records exactly what it changed so a checkpoint restore can rebuild the
/// app from scaffold + addenda — state is derived, never patched.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Addendum {
    pub version: u32,
    pub instruction: String,
    pub reply: String,
    #[serde(default)]
    pub added_feature: Option<String>,
    #[serde(default)]
    pub wired_controls: Vec<String>,
    pub at: u64,
}

/// A scheduled instance of the app. Rendered to a Nomad job on promote;
/// immutable image, short-TTL Vault database credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Allocation {
    pub id: String,
    pub pool: String,
    pub region: String,
    pub image: String,
    pub profile: String,
    pub database: String,
    pub credentials: String,
    pub app_version: u32,
    pub url: String,
    pub healthy: bool,
    pub deployed_at: u64,
    /// Staging (#2): the evaluation id Nomad returned when the rendered job
    /// was really submitted. `None` in simulated mode, so the simulated JSON
    /// shape is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nomad_eval_id: Option<String>,
    /// Staging (#2): the transit key that survived an encrypt/decrypt
    /// round-trip against a real Vault at promote time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_transit_key: Option<String>,
}

/// The attestation a promotion carries: who co-signed, what the gate report
/// said, and the platform reviewer's note (storyboard 1c's co-sign).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attestation {
    pub cosigner: String,
    pub gate_summary: String,
    pub reviewer_note: Option<String>,
    /// The full gate report, frozen verbatim at promotion (F3, review-log
    /// round 1). A released app's compliance record embeds this instead of
    /// re-running preflight over reconstructed sandbox lineage — the report
    /// that admitted the app IS the evidence, basis and stubs included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<crate::gates::GateReport>,
    pub at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppRecord {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub pack: String,
    pub stage: Stage,
    pub data_source: DataSource,
    /// Compliance controls currently satisfied (gate ids). The scaffold
    /// pre-wires some; iteration and "fix it for me" wire the rest.
    pub controls: BTreeSet<String>,
    /// Outbound endpoints the app calls; the ai-allowlist gate checks these.
    pub external_calls: Vec<String>,
    pub features: Vec<String>,
    pub routes: u32,
    pub addenda: Vec<Addendum>,
    pub current_version: u32,
    pub reviewer_note: Option<String>,
    pub allocation: Option<Allocation>,
    pub attestation: Option<Attestation>,
    pub tenant: String,
}

impl AppRecord {
    pub fn version_exists(&self, version: u32) -> bool {
        self.addenda.iter().any(|a| a.version == version)
    }
}

/// Waypoint-style operation status. Running and Escalated are non-terminal:
/// finding one with no terminal successor IS the record of an interrupted
/// action — crash-visibility by construction, not by logging discipline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpStatus {
    Running,
    Success,
    Escalated,
    Failed,
}

/// Which agent verb the operation wraps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpKind {
    Scaffold,
    Iterate,
    Fix,
}

impl OpKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OpKind::Scaffold => "scaffold",
            OpKind::Iterate => "iterate",
            OpKind::Fix => "fix",
        }
    }
}

/// One rung of the escalation ladder: which tier ran, when, and what the
/// verifier said about its output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub tier: String,
    pub started_at: u64,
    pub finished_at: u64,
    /// "accepted" or "rejected" — the verifier's binary call.
    pub verdict: String,
    /// Why a rejected attempt was rejected ("empty-edit",
    /// "gate-regression(auto-logoff lost)", …).
    #[serde(default)]
    pub reason: Option<String>,
}

/// A Waypoint-style operation row: upserted RUNNING before any driver work
/// begins (steering §4), then updated after every attempt. The attempt list
/// is the routing decision, recorded rather than predicted (decision 0001).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub op_id: String,
    pub app_id: String,
    pub kind: OpKind,
    pub status: OpStatus,
    pub attempts: Vec<AttemptRecord>,
    pub started_at: u64,
    pub finished_at: Option<u64>,
}

// TODO(#7): in-memory demo state. Real control plane: Postgres with a
// database-enforced app_valid_state transition table + append-only state
// history (Boundary pattern), and Waypoint-style upsert-first operation rows.
pub struct Platform {
    pub packs: Vec<PackManifest>,
    pub apps: HashMap<String, AppRecord>,
    pub audit: AuditLog,
    /// Waypoint-style operation rows, in creation order. Upsert-first: a row
    /// exists from the moment work is promised, not from when it finishes.
    pub operations: Vec<Operation>,
    /// The escalation ladder the agent supervisor climbs. Built from env at
    /// startup (rules-only when no model endpoints are configured); tests
    /// inject custom ladders here.
    pub ladder: Arc<EscalationLadder>,
    next_id: u64,
}

impl Platform {
    pub fn new(packs: Vec<PackManifest>) -> Self {
        Self {
            packs,
            apps: HashMap::new(),
            audit: AuditLog::default(),
            operations: Vec::new(),
            ladder: Arc::new(EscalationLadder::from_env()),
            next_id: 1,
        }
    }

    /// Insert or replace an operation row by op_id — the Waypoint upsert.
    pub fn upsert_operation(&mut self, op: Operation) {
        match self.operations.iter_mut().find(|o| o.op_id == op.op_id) {
            Some(existing) => *existing = op,
            None => self.operations.push(op),
        }
    }

    pub fn operations_for_app(&self, app_id: &str) -> Vec<&Operation> {
        self.operations
            .iter()
            .filter(|o| o.app_id == app_id)
            .collect()
    }

    pub fn pack(&self, id: &str) -> Option<&PackManifest> {
        self.packs.iter().find(|p| p.id == id)
    }

    /// Short, stable, non-guessable-enough ids for a Phase 0 control plane
    /// (a1f3-style, like a Nomad alloc short id).
    pub fn mint_id(&mut self, prefix: &str) -> String {
        let n = self.next_id;
        self.next_id += 1;
        let mixed = n
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .rotate_left(17)
            .wrapping_add(0x517c_c1b7);
        format!("{prefix}-{:04x}", (mixed % 0xffff) as u16)
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
