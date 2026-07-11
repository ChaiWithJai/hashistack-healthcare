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

impl Stage {
    /// The wire/DB spelling — must match the serde rename and the
    /// `app_valid_state` seed in migrations/0001_init.sql.
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Sandbox => "sandbox",
            Stage::Live => "live",
        }
    }
}

/// The lifecycle transition set, defined ONCE (Boundary's
/// `session_valid_state` pattern, steering §5). Everything that enforces a
/// stage change consults this: `deploy::promote` / `deploy::rollback` in
/// memory via [`valid_transition`], and Postgres via the `app_valid_state`
/// table seeded from these same pairs (a test asserts the SQL seed matches).
pub const VALID_STAGE_TRANSITIONS: &[(Stage, Stage)] = &[
    (Stage::Sandbox, Stage::Live), // promote
    (Stage::Live, Stage::Sandbox), // rollback
];

/// Is `prior → next` a legal lifecycle transition?
pub fn valid_transition(prior: Stage, next: Stage) -> bool {
    VALID_STAGE_TRANSITIONS.contains(&(prior, next))
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
    /// Staging (#9): the database-engine lease behind this allocation's
    /// credentials — the revocation handle. Lease id, username, and TTL are
    /// inspection metadata, not secrets; the password itself is NEVER
    /// stored (see `hashi::DbLease`). `None` in simulated mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_lease_id: Option<String>,
    /// Staging (#9): the Postgres role Vault created for this allocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_db_username: Option<String>,
    /// Staging (#9): the lease TTL in seconds (1h per the tenant-app role).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_lease_ttl_secs: Option<u64>,
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

/// Every operation status, defined once alongside the stage transitions —
/// the `operations.status` CHECK constraint in migrations/0001_init.sql is
/// seeded from these spellings (a test asserts they match).
pub const OP_STATUSES: &[OpStatus] = &[
    OpStatus::Running,
    OpStatus::Success,
    OpStatus::Escalated,
    OpStatus::Failed,
];

impl OpStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OpStatus::Running => "running",
            OpStatus::Success => "success",
            OpStatus::Escalated => "escalated",
            OpStatus::Failed => "failed",
        }
    }
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

// In-memory state is the read path. With `CONTROL_DB_URL` set (#7), every
// mutation writes through to the Postgres control store in src/store.rs —
// database-enforced app_valid_state transitions + append-only state history
// (Boundary pattern, steering §5) and Waypoint upsert-first operation rows
// (§4) — and boot loads it all back. Unset, behavior is byte-identical to
// the pre-#7 in-memory demo.
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
    /// The Postgres control store (#7). `None` (no `CONTROL_DB_URL`) keeps
    /// the platform purely in-memory; `Some` makes every mutation write
    /// through after its lock is released.
    pub store: Option<Arc<crate::store::PgStore>>,
    /// The audit broker (#8): memory-fallback-only in dev; with durable
    /// sinks registered (AUDIT_FILE, control DB), load-bearing operations
    /// require ≥1 durable confirmation — no audit write, no operation.
    pub broker: Arc<crate::audit::Broker>,
    /// Operation rows upserted since the last write-through — tracked only
    /// when a store is attached, drained by [`Platform::take_dirty_operations`].
    dirty_ops: BTreeSet<String>,
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
            store: None,
            broker: Arc::new(crate::audit::Broker::new()),
            dirty_ops: BTreeSet::new(),
            next_id: 1,
        }
    }

    /// Insert or replace an operation row by op_id — the Waypoint upsert.
    pub fn upsert_operation(&mut self, op: Operation) {
        if self.store.is_some() {
            self.dirty_ops.insert(op.op_id.clone());
        }
        match self.operations.iter_mut().find(|o| o.op_id == op.op_id) {
            Some(existing) => *existing = op,
            None => self.operations.push(op),
        }
    }

    /// Drain the operations touched since the last write-through.
    pub fn take_dirty_operations(&mut self) -> Vec<Operation> {
        let ids = std::mem::take(&mut self.dirty_ops);
        self.operations
            .iter()
            .filter(|o| ids.contains(&o.op_id))
            .cloned()
            .collect()
    }

    /// Re-mark operations dirty (a failed write-through must not lose them).
    pub fn remark_dirty_operations(&mut self, ops: &[Operation]) {
        self.dirty_ops.extend(ops.iter().map(|o| o.op_id.clone()));
    }

    /// The id-minting counter — persisted so a restarted control plane never
    /// re-mints an id already used by a loaded record.
    pub fn next_id_counter(&self) -> u64 {
        self.next_id
    }

    pub fn set_next_id_counter(&mut self, n: u64) {
        self.next_id = n.max(1);
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
