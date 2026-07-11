//! Audit pipeline: append-only, and — since #8 — load-bearing.
//!
//! Everything reads from here — the doctor's "who touched what" view and the
//! hospital's security-review export are the same stream. No service may
//! edit or delete an event; there is deliberately no API for it.
//!
//! # The broker invariant (#8, Vault's `audit/` broker — steering §2)
//!
//! Sinks are pluggable backends behind a [`Broker`]. Every sink runs a
//! `LogTestMessage`-style [`AuditSink::probe`] at registration and is
//! rejected loudly at boot if it fails. The in-memory [`MemorySink`] (the
//! [`AuditLog`] Vec every view reads from) is always present and is the
//! **fallback sink** (Vault `IsFallback()`): events are recorded there
//! first, so the record of a durable-sink failure itself is never lost.
//!
//! Policy:
//! - **Dev mode** (no `AUDIT_FILE`, no `CONTROL_DB_URL`): memory alone
//!   counts as durable. Behavior is byte-identical to the pre-#8 demo.
//! - **Any durable sink configured** ([`FileSink`] via `AUDIT_FILE`,
//!   [`crate::store::PgSink`] via `CONTROL_DB_URL`): a *load-bearing*
//!   operation must get ≥1 durable-sink confirmation of its audit events or
//!   the operation fails with 503 audit-unavailable and its state change
//!   reverts — no audit write, no operation. Failed sinks land an
//!   `audit.sink_failed` event in memory and are retried past their own
//!   durable watermark on the next confirmation.
//!
//! # Operation classification
//!
//! Load-bearing (durable confirmation required, state reverts on failure):
//! - `POST /api/apps` — scaffold settle (`app.created`, `agent.scaffolded`)
//! - `POST /api/apps/:id/iterate` — an *applied* edit (`app.iterated`)
//! - `POST /api/apps/:id/gate/:gate/fix` — `gate.fixed`
//! - `POST /api/apps/:id/review` — `review.completed`
//! - `POST /api/apps/:id/promote` — `app.promoted` (additionally requires
//!   the control-DB stage transition itself, #7)
//! - `POST /api/apps/:id/rollback` — `app.rolled_back` (same)
//! - `GET  /api/apps/:id/export` — `app.exported`: the bundle is withheld
//!   unless the record that it left the platform is durable
//!
//! Best-effort (audited or read-only; never blocks the doctor):
//! - reads: gate report, operate view, operations list, audit views
//! - `POST /api/apps/:id/restore` — a sandbox-only derived-state rebuild;
//!   its inputs (scaffold + addenda) were durably recorded by the
//!   load-bearing operations that created them
//! - a *failed* ladder climb keeps its original error; its attempt records
//!   retry into durable sinks on the next confirmation
//!
//! # Salted HMAC for sensitive values (Vault `salt.GetIdentifiedHMAC`)
//!
//! Doctor-authored free text (the describe prompt on `app.created`, the
//! iterate instruction on `app.iterated`) rides [`AuditEvent::sensitive`]
//! as `hmac-sha256:<hex>` — searchable and correlatable, not disclosable.
//! `agent.attempt` events carry only op ids, tiers, and verdict reasons
//! (machine-generated), so nothing there needs the envelope. The key is
//! per-boot random, overridable with `AUDIT_HMAC_KEY` for cross-restart
//! correlation.
//!
//! **The plaintext boundary** (recorded in decision 0004): tenant-scoped
//! surfaces show the doctor their own words — `/api/apps/:id/audit` and the
//! ejected COMPLIANCE.md (it is the doctor's own record). Platform-wide
//! artifacts show only the HMAC form — `/api/audit/export`, the `AUDIT_FILE`
//! archive, and any future cross-tenant view. The control DB stores the
//! Boundary-style pt/HMAC pair ([`AuditEvent::sensitive_pt`] never
//! serializes; it rides its own column) — the same trust domain as
//! `apps.record`, which already carries the prompt — so the tenant view
//! survives a restart.

use std::collections::BTreeMap;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::state::now_unix;

#[derive(Clone, Debug, Serialize)]
pub struct AuditEvent {
    pub seq: u64,
    pub at: u64,
    pub actor: String,
    pub action: String,
    pub detail: String,
    pub app_id: Option<String>,
    /// Sensitive values in their platform-wide form: `hmac-sha256:<hex>`.
    /// This is what every serialization (export JSONL, file archive)
    /// carries — correlatable, not disclosable.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sensitive: BTreeMap<String, String>,
    /// The paired plaintext (Boundary ct/pt pattern) for tenant-scoped
    /// views only. NEVER serialized — the control DB persists it in its own
    /// column, and [`AuditEvent::tenant_sensitive`] merges it back for the
    /// doctor's own view.
    #[serde(skip)]
    pub sensitive_pt: BTreeMap<String, String>,
}

impl AuditEvent {
    /// The sensitive map as the owning tenant sees it: plaintext where the
    /// pair is known, the HMAC form otherwise (an honest degradation, e.g.
    /// an event restored from a store that only kept the HMAC).
    pub fn tenant_sensitive(&self) -> BTreeMap<String, String> {
        self.sensitive
            .iter()
            .map(|(k, hmac)| {
                let v = self.sensitive_pt.get(k).unwrap_or(hmac);
                (k.clone(), v.clone())
            })
            .collect()
    }
}

/// `hmac-sha256:<hex>` of `value` under `key` — Vault's salted-HMAC form.
pub fn hmac_value(key: &[u8; 32], value: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(value.as_bytes());
    let out = mac.finalize().into_bytes();
    let hex: String = out.iter().map(|b| format!("{b:02x}")).collect();
    format!("hmac-sha256:{hex}")
}

/// The per-boot HMAC key: `AUDIT_HMAC_KEY` (hashed to 32 bytes) when set —
/// for cross-restart correlation — else random from /dev/urandom.
fn boot_hmac_key() -> [u8; 32] {
    if let Ok(k) = std::env::var("AUDIT_HMAC_KEY") {
        if !k.trim().is_empty() {
            return Sha256::digest(k.as_bytes()).into();
        }
    }
    let mut buf = [0u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok()
    {
        return buf;
    }
    // No urandom (odd sandbox): still per-boot unique, if weaker.
    Sha256::digest(format!("{}-{}", now_unix(), std::process::id()).as_bytes()).into()
}

// In-memory log = the fallback sink; durable sinks replicate past their own
// watermark via the Broker. TODO(#8) resolved in this link: the broker
// invariant lives in [`Broker`] + `api::settle_durable` — an operation
// fails unless ≥1 durable append-only sink confirms the write, with
// salted-HMAC'd sensitive fields and memory as the fallback sink.
pub struct AuditLog {
    events: Vec<AuditEvent>,
    hmac_key: [u8; 32],
}

impl Default for AuditLog {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            hmac_key: boot_hmac_key(),
        }
    }
}

impl AuditLog {
    /// A log with a fixed HMAC key — tests use this for determinism.
    pub fn with_key(hmac_key: [u8; 32]) -> Self {
        Self {
            events: Vec::new(),
            hmac_key,
        }
    }

    /// Rebuild the log from durable storage at boot (#7). Restore-only —
    /// [`AuditLog::record`] remains the only path that creates a new event.
    /// Keeps the boot HMAC key (restored events carry their stored forms).
    pub fn restore(&mut self, events: Vec<AuditEvent>) {
        self.events = events;
    }

    /// The only write path. Returns the sequence number as a receipt.
    pub fn record(
        &mut self,
        actor: &str,
        action: &str,
        detail: impl Into<String>,
        app_id: Option<&str>,
    ) -> u64 {
        self.record_sensitive(actor, action, detail, app_id, &[])
    }

    /// Record an event carrying doctor-authored free text: each `(key,
    /// plaintext)` pair is stored as its salted HMAC in the serialized
    /// event, with the plaintext paired alongside for tenant-scoped views.
    pub fn record_sensitive(
        &mut self,
        actor: &str,
        action: &str,
        detail: impl Into<String>,
        app_id: Option<&str>,
        sensitive: &[(&str, String)],
    ) -> u64 {
        let seq = self.events.last().map(|e| e.seq).unwrap_or(0) + 1;
        let mut hmacs = BTreeMap::new();
        let mut pts = BTreeMap::new();
        for (key, plaintext) in sensitive {
            hmacs.insert(key.to_string(), hmac_value(&self.hmac_key, plaintext));
            pts.insert(key.to_string(), plaintext.clone());
        }
        self.events.push(AuditEvent {
            seq,
            at: now_unix(),
            actor: actor.to_string(),
            action: action.to_string(),
            detail: detail.into(),
            app_id: app_id.map(str::to_string),
            sensitive: hmacs,
            sensitive_pt: pts,
        });
        seq
    }

    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    /// Highest sequence number recorded — the broker's confirmation target.
    pub fn head_seq(&self) -> u64 {
        self.events.last().map(|e| e.seq).unwrap_or(0)
    }

    pub fn for_app(&self, app_id: &str) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.app_id.as_deref() == Some(app_id))
            .collect()
    }

    /// The tenant-scoped rendering of one app's stream: identical to
    /// [`AuditLog::for_app`] except `sensitive` shows the doctor their own
    /// plaintext. This is the ONLY surface that crosses the HMAC boundary,
    /// and only within the owning tenant's app view.
    pub fn for_app_tenant_view(&self, app_id: &str) -> Vec<serde_json::Value> {
        self.for_app(app_id)
            .into_iter()
            .map(|e| {
                let mut v = serde_json::to_value(e).expect("audit event serializes");
                if !e.sensitive.is_empty() {
                    v["sensitive"] = serde_json::to_value(e.tenant_sensitive())
                        .expect("sensitive map serializes");
                }
                v
            })
            .collect()
    }

    /// JSON-lines export for a security review — one event per line, in
    /// sequence order, suitable for diffing against a prior export. This is
    /// the platform-wide surface: sensitive values appear ONLY in their
    /// `hmac-sha256:` form (plaintext is `#[serde(skip)]`).
    pub fn export_jsonl(&self) -> String {
        self.events
            .iter()
            .map(|e| serde_json::to_string(e).expect("audit event serializes"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ---------- sinks ----------

pub type SinkFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// One audit backend behind the broker (Vault `audit.Backend`). `Ok(())`
/// from [`AuditSink::append`] means *durably confirmed* (fsync / commit),
/// never merely buffered.
pub trait AuditSink: Send + Sync {
    fn name(&self) -> &'static str;
    /// Whether this sink's confirmation satisfies the broker invariant.
    /// The memory fallback returns false: it counts only in dev mode, where
    /// no durable sink is configured at all.
    fn durable(&self) -> bool;
    /// The fallback sink catches what durable pipelines miss (`IsFallback`).
    fn is_fallback(&self) -> bool {
        false
    }
    /// Highest seq this sink has durably confirmed — its retry watermark.
    fn confirmed_seq(&self) -> u64;
    /// `LogTestMessage`-style self-test, run at registration. A sink that
    /// fails it is rejected loudly at boot.
    fn probe(&self) -> SinkFuture<'_>;
    /// Append every event newer than the sink's own watermark, durably.
    fn append<'a>(&'a self, events: &'a [AuditEvent]) -> SinkFuture<'a>;
}

/// The always-present fallback: the [`AuditLog`] Vec itself. Events are
/// born there under the platform lock, so by the time the broker runs they
/// are already recorded — including the record of any durable-sink failure.
pub struct MemorySink;

impl AuditSink for MemorySink {
    fn name(&self) -> &'static str {
        "memory"
    }
    fn durable(&self) -> bool {
        false
    }
    fn is_fallback(&self) -> bool {
        true
    }
    fn confirmed_seq(&self) -> u64 {
        u64::MAX // everything recorded is, by construction, in memory
    }
    fn probe(&self) -> SinkFuture<'_> {
        Box::pin(async { Ok(()) })
    }
    fn append<'a>(&'a self, _events: &'a [AuditEvent]) -> SinkFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

/// JSONL archive sink (`AUDIT_FILE`): line-per-event, fsync on every append
/// — `Ok` means the bytes reached the disk, not the page cache. The file
/// carries the platform-wide (HMAC) form of every event.
pub struct FileSink {
    path: PathBuf,
    file: Mutex<std::fs::File>,
    confirmed: AtomicU64,
}

impl FileSink {
    /// Open (creating if absent) and reconcile the durable watermark:
    /// `min(max seq already in the file, max seq restored into memory)`, so
    /// a control plane restored from the control DB never re-appends events
    /// the archive already holds, while a fresh in-memory boot (seq restarts
    /// at 1) starts a new appended stream — the registration probe line
    /// timestamps each boot's stream.
    pub fn open(path: impl Into<PathBuf>, restored_seq: u64) -> anyhow::Result<Self> {
        let path = path.into();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)
            .with_context(|| format!("opening AUDIT_FILE at {}", path.display()))?;
        file.seek(SeekFrom::Start(0))?;
        let mut existing = String::new();
        file.read_to_string(&mut existing)
            .with_context(|| format!("reading back AUDIT_FILE at {}", path.display()))?;
        let in_file = existing
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v.get("seq").and_then(serde_json::Value::as_u64))
            .max()
            .unwrap_or(0);
        Ok(Self {
            path,
            file: Mutex::new(file),
            confirmed: AtomicU64::new(in_file.min(restored_seq)),
        })
    }

    fn write_durably(&self, buf: &str) -> anyhow::Result<()> {
        let mut file = self.file.lock().unwrap();
        file.write_all(buf.as_bytes())
            .with_context(|| format!("appending to {}", self.path.display()))?;
        file.sync_all()
            .with_context(|| format!("fsync on {}", self.path.display()))?;
        Ok(())
    }
}

impl AuditSink for FileSink {
    fn name(&self) -> &'static str {
        "file"
    }
    fn durable(&self) -> bool {
        true
    }
    fn confirmed_seq(&self) -> u64 {
        self.confirmed.load(Ordering::SeqCst)
    }
    /// Writes a probe line (seq 0, `audit.sink_probe`) with fsync, then
    /// reads the file back and verifies the line landed — write AND read
    /// proven before the sink is accepted.
    fn probe(&self) -> SinkFuture<'_> {
        Box::pin(async move {
            let marker = format!(
                "{{\"seq\":0,\"at\":{},\"actor\":\"audit-broker\",\
                 \"action\":\"audit.sink_probe\",\"detail\":\
                 \"registration self-test — one line per boot stream\",\"app_id\":null}}",
                now_unix()
            );
            self.write_durably(&format!("{marker}\n"))?;
            let mut file = self.file.lock().unwrap();
            file.seek(SeekFrom::Start(0))?;
            let mut readback = String::new();
            file.read_to_string(&mut readback)?;
            anyhow::ensure!(
                readback.contains(&marker),
                "probe line written to {} did not read back",
                self.path.display()
            );
            Ok(())
        })
    }
    fn append<'a>(&'a self, events: &'a [AuditEvent]) -> SinkFuture<'a> {
        Box::pin(async move {
            let since = self.confirmed.load(Ordering::SeqCst);
            let pending: Vec<&AuditEvent> = events.iter().filter(|e| e.seq > since).collect();
            let Some(last) = pending.last() else {
                return Ok(());
            };
            let max_seq = last.seq;
            let mut buf = String::new();
            for e in &pending {
                buf.push_str(&serde_json::to_string(e).expect("audit event serializes"));
                buf.push('\n');
            }
            // Small synchronous write+fsync; watermark advances only after
            // the fsync succeeded, so a failed append retries from the gap.
            self.write_durably(&buf)?;
            self.confirmed.fetch_max(max_seq, Ordering::SeqCst);
            Ok(())
        })
    }
}

// ---------- the broker ----------

/// One durable sink's verdict on a confirmation round.
#[derive(Default)]
pub struct Confirmation {
    /// Durable sinks whose watermark reached the target after this round.
    pub confirmed: Vec<&'static str>,
    /// Durable sinks that failed, with why — each becomes an
    /// `audit.sink_failed` fallback event.
    pub failed: Vec<(&'static str, String)>,
}

/// Vault's success-threshold broker: owns the sinks, probes at
/// registration, and answers "did ≥1 durable sink confirm through seq N?".
pub struct Broker {
    sinks: Vec<Arc<dyn AuditSink>>,
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

impl Broker {
    /// A broker with only the memory fallback — dev mode.
    pub fn new() -> Self {
        Self {
            sinks: vec![Arc::new(MemorySink)],
        }
    }

    /// Probe-then-accept (Vault `LogTestMessage`): a sink failing its
    /// registration self-test is rejected here, which fails the boot.
    pub async fn register(&mut self, sink: Arc<dyn AuditSink>) -> anyhow::Result<()> {
        sink.probe().await.with_context(|| {
            format!(
                "audit sink {:?} failed its registration probe — rejected",
                sink.name()
            )
        })?;
        self.sinks.push(sink);
        Ok(())
    }

    pub fn sink_names(&self) -> Vec<&'static str> {
        self.sinks.iter().map(|s| s.name()).collect()
    }

    /// Is any durable sink configured? False = dev mode: memory (the
    /// fallback) alone suffices and nothing is ever blocked on durability.
    pub fn durable_configured(&self) -> bool {
        self.sinks.iter().any(|s| s.durable())
    }

    /// One confirmation round: every durable sink appends past its own
    /// watermark; it confirms iff its watermark then covers `target_seq`.
    pub async fn confirm(&self, events: &[AuditEvent], target_seq: u64) -> Confirmation {
        let mut outcome = Confirmation::default();
        for sink in self.sinks.iter().filter(|s| s.durable()) {
            match sink.append(events).await {
                Ok(()) if sink.confirmed_seq() >= target_seq => {
                    outcome.confirmed.push(sink.name());
                }
                Ok(()) => outcome.failed.push((
                    sink.name(),
                    format!(
                        "confirmed only through seq {} of {target_seq}",
                        sink.confirmed_seq()
                    ),
                )),
                Err(e) => outcome.failed.push((sink.name(), format!("{e:#}"))),
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    #[test]
    fn hmac_is_deterministic_per_key_and_keyed() {
        let a = hmac_value(&KEY, "remind patients to log wound photos");
        let b = hmac_value(&KEY, "remind patients to log wound photos");
        assert_eq!(a, b, "same key + text must correlate");
        assert!(a.starts_with("hmac-sha256:"));
        let other = hmac_value(&[8u8; 32], "remind patients to log wound photos");
        assert_ne!(a, other, "a different salt must decorrelate");
        assert_ne!(a, hmac_value(&KEY, "different text"));
    }

    #[test]
    fn export_carries_only_the_hmac_form_and_tenant_view_the_plaintext() {
        let mut log = AuditLog::with_key(KEY);
        log.record_sensitive(
            "dr-osei",
            "app.created",
            "described from pack post-op-monitor",
            Some("app-1"),
            &[("prompt", "my knee replacement patients".to_string())],
        );

        let export = log.export_jsonl();
        assert!(export.contains("hmac-sha256:"), "{export}");
        assert!(
            !export.contains("knee replacement"),
            "platform export must never disclose the prompt: {export}"
        );

        let view = log.for_app_tenant_view("app-1");
        let sensitive = &view[0]["sensitive"];
        assert_eq!(
            sensitive["prompt"], "my knee replacement patients",
            "the doctor's own view shows their own words"
        );
    }

    #[test]
    fn tenant_view_degrades_to_hmac_when_plaintext_was_not_restored() {
        let mut log = AuditLog::with_key(KEY);
        log.record_sensitive("a", "x", "d", Some("app-1"), &[("k", "v".to_string())]);
        let mut events = log.events().to_vec();
        events[0].sensitive_pt.clear(); // as if restored from an HMAC-only store
        let mut restored = AuditLog::with_key(KEY);
        restored.restore(events);
        let view = restored.for_app_tenant_view("app-1");
        assert_eq!(view[0]["sensitive"]["k"], hmac_value(&KEY, "v"));
    }

    #[test]
    fn sequence_numbers_stay_strictly_increasing_after_restore() {
        let mut log = AuditLog::with_key(KEY);
        log.record("a", "one", "d", None);
        log.record("a", "two", "d", None);
        let events = log.events().to_vec();
        let mut restored = AuditLog::with_key(KEY);
        restored.restore(events);
        let seq = restored.record("a", "three", "d", None);
        assert_eq!(seq, 3, "restore must not reset the receipt counter");
    }

    #[tokio::test]
    async fn file_sink_probe_writes_and_reads_back_and_append_fsyncs_jsonl() {
        let path =
            std::env::temp_dir().join(format!("audit-sink-test-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let sink = FileSink::open(&path, 0).expect("open");
        sink.probe().await.expect("probe self-test");

        let mut log = AuditLog::with_key(KEY);
        log.record_sensitive(
            "dr",
            "app.created",
            "d",
            Some("a"),
            &[("prompt", "words".into())],
        );
        sink.append(log.events()).await.expect("append");
        assert_eq!(sink.confirmed_seq(), 1);
        // Idempotent past the watermark.
        sink.append(log.events()).await.expect("re-append no-ops");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("audit.sink_probe"));
        assert!(content.contains("hmac-sha256:"), "{content}");
        assert!(
            !content.contains("words"),
            "archive is platform-wide: {content}"
        );
        assert_eq!(content.lines().count(), 2, "probe line + one event");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn file_sink_resumes_its_watermark_from_the_archive() {
        let path =
            std::env::temp_dir().join(format!("audit-sink-resume-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut log = AuditLog::with_key(KEY);
        log.record("a", "one", "d", None);
        log.record("a", "two", "d", None);
        {
            let sink = FileSink::open(&path, 0).expect("open");
            sink.append(log.events()).await.unwrap();
        }
        // Restart restored 2 events from the control DB: nothing re-appends.
        let sink = FileSink::open(&path, log.head_seq()).expect("reopen");
        assert_eq!(sink.confirmed_seq(), 2);
        sink.append(log.events()).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().lines().count(),
            2,
            "no duplicate lines after a restored reboot"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_sink_that_cannot_open_its_path_is_rejected_at_registration() {
        let err = match FileSink::open("/nonexistent-dir-for-audit-test/audit.jsonl", 0) {
            Ok(_) => panic!("open must fail"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("AUDIT_FILE"), "{err}");
    }

    #[tokio::test]
    async fn broker_requires_the_probe_before_accepting_a_sink() {
        struct DeadOnArrival;
        impl AuditSink for DeadOnArrival {
            fn name(&self) -> &'static str {
                "doa"
            }
            fn durable(&self) -> bool {
                true
            }
            fn confirmed_seq(&self) -> u64 {
                0
            }
            fn probe(&self) -> SinkFuture<'_> {
                Box::pin(async { anyhow::bail!("no medium") })
            }
            fn append<'a>(&'a self, _e: &'a [AuditEvent]) -> SinkFuture<'a> {
                Box::pin(async { Ok(()) })
            }
        }
        let mut broker = Broker::new();
        let err = broker
            .register(Arc::new(DeadOnArrival))
            .await
            .expect_err("a failing probe must reject the sink");
        assert!(err.to_string().contains("registration probe"), "{err}");
        assert!(!broker.durable_configured(), "the sink must not be kept");
    }
}
