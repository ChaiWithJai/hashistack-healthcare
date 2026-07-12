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
    /// A durable rollback intent exists. `cleanup_workload_stopped` records
    /// the separately confirmed irreversible Nomad step.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cleanup_pending: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub cleanup_workload_stopped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_error: Option<String>,
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

impl Allocation {
    pub fn validate_cleanup_state(&self) -> Result<(), String> {
        if self.cleanup_workload_stopped && !self.cleanup_pending {
            return Err("cleanup_workload_stopped requires cleanup_pending".to_string());
        }
        if self.cleanup_error.is_some() && !self.cleanup_pending {
            return Err("cleanup_error requires cleanup_pending".to_string());
        }
        Ok(())
    }

    pub fn has_external_cleanup_handles(&self) -> bool {
        self.nomad_eval_id.is_some() || self.vault_lease_id.is_some()
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The attestation a promotion carries: who co-signed, what the gate report
/// said, and the platform reviewer's note (storyboard 1c's co-sign).
///
/// #10 makes the co-sign cryptographic: `principal` is the authenticated
/// clinician the control plane resolved (never a typed claim), `cosigner`
/// is their display name, and `report_digest` is a sha256 over the frozen
/// report's canonical JSON — the signature binds identity plus the exact
/// evidence plus the timestamp. Both new fields are `Option` so records
/// promoted before #10 deserialize unchanged (honestly absent, never
/// backfilled).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attestation {
    /// Display name of the co-signing clinician — always the authenticated
    /// principal's registered name (a mismatched typed claim is refused).
    pub cosigner: String,
    /// The authenticated principal id whose act this attestation records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
    pub gate_summary: String,
    pub reviewer_note: Option<String>,
    /// The full gate report, frozen verbatim at promotion (F3, review-log
    /// round 1). A released app's compliance record embeds this instead of
    /// re-running preflight over reconstructed sandbox lineage — the report
    /// that admitted the app IS the evidence, basis and stubs included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<crate::gates::GateReport>,
    /// `sha256:<hex>` over the frozen report's canonical JSON
    /// ([`crate::gates::report_digest`]) — what the co-sign binds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_digest: Option<String>,
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

impl Operation {
    /// Close work that a previous control-plane process promised but never
    /// completed. Boot performs this reconciliation before serving traffic,
    /// so `running` never becomes a permanent zombie after a crash.
    pub fn interrupt_on_restart(&mut self, at: u64) -> bool {
        if !matches!(self.status, OpStatus::Running | OpStatus::Escalated) {
            return false;
        }
        self.status = OpStatus::Failed;
        self.finished_at = Some(at);
        self.attempts.push(AttemptRecord {
            tier: "control-plane".to_string(),
            started_at: at,
            finished_at: at,
            verdict: "rejected".to_string(),
            reason: Some("control-plane-restart".to_string()),
        });
        true
    }
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
    /// Injectable cleanup boundary: production delegates to HashiStack;
    /// contract tests inject stop/revoke failures without mutating process
    /// environment or pretending a backend call happened.
    pub cleanup_driver: Arc<dyn crate::deploy::CleanupDriver>,
    /// The identity registry (#10): who may call `/api` routes, as which
    /// principal. Defaults to the embedded dev registry (dr-osei fallback);
    /// `app_from_env` swaps in `IDENTITIES_FILE` / `SESSION_IDLE_SECS`.
    pub identity: Arc<crate::identity::Registry>,
    pub clerk: Option<Arc<crate::clerk::ClerkVerifier>>,
    pub anonymous: Arc<crate::anonymous::AnonymousSessions>,
    /// Operation rows upserted since the last write-through — tracked only
    /// when a store is attached, drained by [`Platform::take_dirty_operations`].
    dirty_ops: BTreeSet<String>,
    /// Mutations for one app are serialized through durable settlement. The
    /// global platform lock remains short-lived; this per-app async lock
    /// prevents a later successful edit from depending on an earlier edit
    /// whose audit confirmation is still pending.
    app_locks: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    pending_app_ids: BTreeSet<String>,
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
            cleanup_driver: Arc::new(crate::deploy::HashiCleanupDriver),
            identity: Arc::new(crate::identity::Registry::dev_default()),
            clerk: None,
            anonymous: Arc::new(crate::anonymous::AnonymousSessions::development()),
            dirty_ops: BTreeSet::new(),
            app_locks: HashMap::new(),
            pending_app_ids: BTreeSet::new(),
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

    pub fn app_lock(&mut self, app_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.app_locks
            .entry(app_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    pub fn reserve_app_id(&mut self, preferred: &str) -> String {
        let mut candidate = preferred.to_string();
        while self.apps.contains_key(&candidate)
            || self.pending_app_ids.contains(&candidate)
            || self.operations.iter().any(|op| op.app_id == candidate)
        {
            candidate = self.mint_id(preferred);
        }
        self.pending_app_ids.insert(candidate.clone());
        candidate
    }

    pub fn release_app_id(&mut self, id: &str) {
        self.pending_app_ids.remove(id);
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

#[cfg(test)]
mod operation_recovery_tests {
    use super::*;

    fn operation(status: OpStatus) -> Operation {
        Operation {
            op_id: "op-restart".to_string(),
            app_id: "app-1".to_string(),
            kind: OpKind::Iterate,
            status,
            attempts: Vec::new(),
            started_at: 10,
            finished_at: None,
        }
    }

    #[test]
    fn restart_closes_running_operation_as_failed() {
        let mut op = operation(OpStatus::Running);
        assert!(op.interrupt_on_restart(42));
        assert_eq!(op.status, OpStatus::Failed);
        assert_eq!(op.finished_at, Some(42));
        assert_eq!(op.attempts.len(), 1);
        assert_eq!(op.attempts[0].tier, "control-plane");
        assert_eq!(
            op.attempts[0].reason.as_deref(),
            Some("control-plane-restart")
        );
    }

    #[test]
    fn restart_does_not_rewrite_terminal_operation() {
        let mut op = operation(OpStatus::Success);
        assert!(!op.interrupt_on_restart(42));
        assert_eq!(op.status, OpStatus::Success);
        assert!(op.finished_at.is_none());
        assert!(op.attempts.is_empty());
    }
}

#[cfg(test)]
mod allocation_cleanup_tests {
    use super::Allocation;

    fn allocation() -> Allocation {
        Allocation {
            id: "a".into(),
            pool: "prod".into(),
            region: "nyc3".into(),
            image: "image@sha256:test".into(),
            profile: "small".into(),
            database: "dynamic".into(),
            credentials: "lease".into(),
            app_version: 1,
            url: "https://example.invalid".into(),
            healthy: false,
            cleanup_pending: false,
            cleanup_workload_stopped: false,
            cleanup_error: None,
            deployed_at: 1,
            nomad_eval_id: None,
            vault_transit_key: None,
            vault_lease_id: None,
            vault_db_username: None,
            vault_lease_ttl_secs: None,
        }
    }

    #[test]
    fn contradictory_cleanup_states_are_rejected() {
        let mut stopped_without_intent = allocation();
        stopped_without_intent.cleanup_workload_stopped = true;
        assert!(stopped_without_intent.validate_cleanup_state().is_err());

        let mut error_without_intent = allocation();
        error_without_intent.cleanup_error = Some("failure".into());
        assert!(error_without_intent.validate_cleanup_state().is_err());

        let mut valid = allocation();
        valid.cleanup_pending = true;
        valid.cleanup_workload_stopped = true;
        valid.cleanup_error = Some("credential-cleanup-failed".into());
        assert!(valid.validate_cleanup_state().is_ok());
    }
}
