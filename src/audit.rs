//! Audit pipeline: append-only. Everything reads from here — the doctor's
//! "who touched what" view and the hospital's security-review export are the
//! same stream. No service may edit or delete an event; there is deliberately
//! no API for it.

use serde::Serialize;

use crate::state::now_unix;

#[derive(Clone, Debug, Serialize)]
pub struct AuditEvent {
    pub seq: u64,
    pub at: u64,
    pub actor: String,
    pub action: String,
    pub detail: String,
    pub app_id: Option<String>,
}

// TODO(#8): in-memory demo sink. With CONTROL_DB_URL set (#7) events write
// through to the append-only audit_events table (INSERT-only triggers), but
// the real pipeline still adopts Vault's broker invariant — an operation
// fails unless ≥1 durable append-only sink confirms the write — with
// salted-HMAC'd sensitive fields and a fallback sink. #7 applies that
// invariant to stage transitions only.
#[derive(Default)]
pub struct AuditLog {
    events: Vec<AuditEvent>,
}

impl AuditLog {
    /// Rebuild the log from durable storage at boot (#7). Restore-only —
    /// [`AuditLog::record`] remains the only path that creates a new event.
    pub fn restore(events: Vec<AuditEvent>) -> Self {
        Self { events }
    }

    /// The only write path. Returns the sequence number as a receipt.
    pub fn record(
        &mut self,
        actor: &str,
        action: &str,
        detail: impl Into<String>,
        app_id: Option<&str>,
    ) -> u64 {
        let seq = self.events.len() as u64 + 1;
        self.events.push(AuditEvent {
            seq,
            at: now_unix(),
            actor: actor.to_string(),
            action: action.to_string(),
            detail: detail.into(),
            app_id: app_id.map(str::to_string),
        });
        seq
    }

    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    pub fn for_app(&self, app_id: &str) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.app_id.as_deref() == Some(app_id))
            .collect()
    }

    /// JSON-lines export for a security review — one event per line, in
    /// sequence order, suitable for diffing against a prior export.
    pub fn export_jsonl(&self) -> String {
        self.events
            .iter()
            .map(|e| serde_json::to_string(e).expect("audit event serializes"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
