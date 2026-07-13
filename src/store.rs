//! Postgres control store (#7): durability behind the in-memory read path.
//!
//! `CONTROL_DB_URL` unset → this module is never constructed and the
//! platform is byte-identical to the pre-#7 in-memory demo. Set, every
//! mutation writes through AFTER its platform lock is released (write
//! locks stay short — F4), and boot loads the whole state back: apps,
//! editable source workspaces, operations, audit stream, and the id-minting counter all survive a
//! `kill -9`.
//!
//! Enforcement lives in the database, not only in this code
//! (migrations/0001_init.sql): a trigger rejects any `apps.stage` change
//! not present in `app_valid_state` (Boundary §5), the state-history and
//! audit tables are append-only by trigger, and the history's composite FK
//! makes an illegal recorded transition unrepresentable. The
//! `app_valid_state` seed comes from the SAME const as the in-memory check
//! (`state::VALID_STAGE_TRANSITIONS`) — tests/store_contract.rs asserts
//! they never drift.
//!
//! Failure policy: a failed write on a STAGE TRANSITION fails the operation
//! — the caller reverts the in-memory record and returns 503. Failed writes
//! elsewhere degrade durability, are logged loudly, and re-mark their rows
//! dirty for the next write-through. Since #8 the broker invariant extends
//! this to every load-bearing operation's AUDIT record: [`PgSink`] exposes
//! the `audit_events` table as a broker sink (src/audit.rs), and
//! `api::settle_durable` fails the operation when no durable sink confirms.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio_postgres::NoTls;

use crate::audit::AuditEvent;
use crate::state::{AppRecord, Operation, Platform, SharedPlatform, Stage};

/// The idempotent schema, applied on every boot.
pub const MIGRATION: &str = include_str!("../migrations/0001_init.sql");

/// One lifecycle stage change, recorded in `app_state_history` in the same
/// transaction as the app row it changes. `prior: None` is creation.
pub struct StageTransition {
    pub app_id: String,
    pub prior: Option<Stage>,
    pub next: Stage,
    pub operation_id: Option<String>,
}

/// Everything one write-through carries, snapshotted under the platform
/// lock and written with no lock held.
struct PersistBatch {
    apps: Vec<AppRecord>,
    workspaces: Vec<crate::workspace::WorkspaceRecord>,
    transition: Option<StageTransition>,
    operations: Vec<Operation>,
    audit: Vec<AuditEvent>,
    next_id: u64,
}

pub struct PgStore {
    /// tokio-postgres client behind an async Mutex: writes are serialized
    /// (one control plane, small scale) and `transaction()` needs `&mut`.
    client: tokio::sync::Mutex<tokio_postgres::Client>,
    /// Highest audit seq known durable — write-through appends past this.
    persisted_seq: AtomicU64,
}

impl PgStore {
    /// Connect and apply migrations/0001_init.sql (idempotent). Plain TCP —
    /// the control DB is loopback/in-VPC by definition, like the model tiers.
    pub async fn connect(url: &str) -> Result<Self> {
        let (client, connection) = tokio_postgres::connect(url, NoTls)
            .await
            .with_context(|| format!("connecting to control DB at {url}"))?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::error!("control DB connection lost: {e}");
            }
        });
        client
            .batch_execute(MIGRATION)
            .await
            .context("applying migrations/0001_init.sql")?;
        Ok(Self {
            client: tokio::sync::Mutex::new(client),
            persisted_seq: AtomicU64::new(0),
        })
    }

    /// Load the durable state back into a fresh platform at boot. Returns
    /// (apps, workspaces, operations, audit events) restored, for the boot log.
    pub async fn load(&self, plat: &mut Platform) -> Result<(usize, usize, usize, usize)> {
        let mut client = self.client.lock().await;

        for row in client.query("SELECT record FROM apps", &[]).await? {
            let record: serde_json::Value = row.get(0);
            let app: AppRecord =
                serde_json::from_value(record).context("apps.record does not parse")?;
            if let Some(allocation) = app.allocation.as_ref() {
                allocation
                    .validate_cleanup_state()
                    .map_err(anyhow::Error::msg)
                    .context("apps.record has contradictory cleanup state")?;
            }
            plat.apps.insert(app.id.clone(), app);
        }

        for row in client
            .query("SELECT app_id, record FROM source_workspaces", &[])
            .await?
        {
            let app_id: String = row.get(0);
            let record: serde_json::Value = row.get(1);
            let workspace: crate::workspace::WorkspaceRecord = serde_json::from_value(record)
                .with_context(|| format!("source_workspaces.record does not parse for {app_id}"))?;
            if workspace.app_id != app_id || !plat.apps.contains_key(&app_id) {
                anyhow::bail!("source workspace {app_id} does not match a durable app");
            }
            workspace
                .validate_restored()
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("source workspace {app_id} failed integrity checks"))?;
            plat.workspaces.insert(app_id, workspace);
        }

        // One-time compatibility path for apps written before source workspace
        // durability existed. We can faithfully reconstruct the pack scaffold,
        // but never pretend to recover edits that were only held in memory.
        // Persist the v0 record before serving so the migration happens once.
        let missing: Vec<String> = plat
            .apps
            .keys()
            .filter(|id| !plat.workspaces.contains_key(*id))
            .cloned()
            .collect();
        if !missing.is_empty() {
            let tx = client.transaction().await?;
            for app_id in missing {
                let app = plat.apps.get(&app_id).expect("missing id came from apps");
                let pack = plat
                    .packs
                    .iter()
                    .find(|pack| pack.id == app.pack)
                    .with_context(|| {
                        format!(
                            "cannot reconstruct workspace {app_id}: pack {} is unavailable",
                            app.pack
                        )
                    })?;
                let workspace = crate::workspace::WorkspaceRecord::new(
                    app_id.clone(),
                    crate::eject::bundle(app, pack, &[]).files,
                    crate::state::now_unix(),
                );
                workspace
                    .validate_restored()
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("reconstructed workspace {app_id} is invalid"))?;
                let record = serde_json::to_value(&workspace)
                    .context("reconstructed workspace serializes")?;
                tx.execute(
                    "INSERT INTO source_workspaces (app_id, record, updated_at) VALUES ($1, $2, $3)",
                    &[&app_id, &record, &(workspace.updated_at as i64)],
                )
                .await
                .with_context(|| format!("persisting reconstructed workspace {app_id}"))?;
                tracing::warn!(
                    "reconstructed v0 source workspace for legacy app {app_id}; pre-migration in-memory edits were not recoverable"
                );
                plat.workspaces.insert(app_id, workspace);
            }
            tx.commit()
                .await
                .context("committing legacy workspace reconstruction")?;
        }

        for row in client
            .query("SELECT record FROM operations ORDER BY ord", &[])
            .await?
        {
            let record: serde_json::Value = row.get(0);
            let mut op: Operation =
                serde_json::from_value(record).context("operations.record does not parse")?;
            if op.interrupt_on_restart(crate::state::now_unix()) {
                let updated =
                    serde_json::to_value(&op).context("serializing interrupted operation")?;
                client
                    .execute(
                        "UPDATE operations SET status = 'failed', record = $2, finished_at = $3 WHERE op_id = $1",
                        &[&op.op_id, &updated, &(op.finished_at.unwrap_or_default() as i64)],
                    )
                    .await
                    .with_context(|| format!("reconciling interrupted operation {}", op.op_id))?;
            }
            plat.operations.push(op);
        }

        let mut events = Vec::new();
        for row in client
            .query(
                "SELECT seq, at, actor, action, detail, app_id, sensitive, sensitive_pt \
                 FROM audit_events ORDER BY seq",
                &[],
            )
            .await?
        {
            let sensitive: serde_json::Value = row.get(6);
            let sensitive_pt: serde_json::Value = row.get(7);
            events.push(AuditEvent {
                seq: row.get::<_, i64>(0) as u64,
                at: row.get::<_, i64>(1) as u64,
                actor: row.get(2),
                action: row.get(3),
                detail: row.get(4),
                app_id: row.get(5),
                sensitive: serde_json::from_value(sensitive)
                    .context("audit_events.sensitive does not parse")?,
                sensitive_pt: serde_json::from_value(sensitive_pt)
                    .context("audit_events.sensitive_pt does not parse")?,
            });
        }
        let max_seq = events.last().map(|e| e.seq).unwrap_or(0);
        let counts = (
            plat.apps.len(),
            plat.workspaces.len(),
            plat.operations.len(),
            events.len(),
        );
        plat.audit.restore(events);
        self.persisted_seq.store(max_seq, Ordering::SeqCst);

        if let Some(row) = client
            .query_opt("SELECT value FROM control_meta WHERE key = 'next_id'", &[])
            .await?
        {
            plat.set_next_id_counter(row.get::<_, i64>(0) as u64);
        }
        Ok(counts)
    }

    /// Write one batch in a single transaction. The stage-transition
    /// history row rides the same transaction as its app upsert, so the DB
    /// trigger + FK verdict is atomic with the record change.
    async fn persist(&self, batch: &PersistBatch) -> Result<()> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;

        for app in &batch.apps {
            let record = serde_json::to_value(app).context("app record serializes")?;
            tx.execute(
                "INSERT INTO apps (app_id, stage, record, updated_at) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (app_id) DO UPDATE SET \
                   stage = EXCLUDED.stage, record = EXCLUDED.record, \
                   updated_at = EXCLUDED.updated_at",
                &[
                    &app.id,
                    &app.stage.as_str(),
                    &record,
                    &(crate::state::now_unix() as i64),
                ],
            )
            .await
            .with_context(|| format!("upserting app {}", app.id))?;
        }

        for workspace in &batch.workspaces {
            let record = serde_json::to_value(workspace).context("workspace record serializes")?;
            tx.execute(
                "INSERT INTO source_workspaces (app_id, record, updated_at) \
                 VALUES ($1, $2, $3) \
                 ON CONFLICT (app_id) DO UPDATE SET \
                   record = EXCLUDED.record, updated_at = EXCLUDED.updated_at",
                &[&workspace.app_id, &record, &(workspace.updated_at as i64)],
            )
            .await
            .with_context(|| format!("upserting source workspace {}", workspace.app_id))?;
        }

        if let Some(t) = &batch.transition {
            tx.execute(
                "INSERT INTO app_state_history \
                   (app_id, prior_state, current_state, at, operation_id) \
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &t.app_id,
                    &t.prior.map(|s| s.as_str()),
                    &t.next.as_str(),
                    &(crate::state::now_unix() as i64),
                    &t.operation_id,
                ],
            )
            .await
            .with_context(|| format!("recording state history for app {}", t.app_id))?;
        }

        for op in &batch.operations {
            let record = serde_json::to_value(op).context("operation serializes")?;
            tx.execute(
                "INSERT INTO operations \
                   (op_id, app_id, kind, status, record, started_at, finished_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (op_id) DO UPDATE SET \
                   status = EXCLUDED.status, record = EXCLUDED.record, \
                   finished_at = EXCLUDED.finished_at",
                &[
                    &op.op_id,
                    &op.app_id,
                    &op.kind.as_str(),
                    &op.status.as_str(),
                    &record,
                    &(op.started_at as i64),
                    &op.finished_at.map(|t| t as i64),
                ],
            )
            .await
            .with_context(|| format!("upserting operation {}", op.op_id))?;
        }

        let mut max_seq = 0u64;
        for event in &batch.audit {
            insert_audit_event(&tx, event).await?;
            max_seq = max_seq.max(event.seq);
        }

        tx.execute(
            "INSERT INTO control_meta (key, value) VALUES ('next_id', $1) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
            &[&(batch.next_id as i64)],
        )
        .await?;

        tx.commit().await.context("committing write-through")?;
        self.persisted_seq.fetch_max(max_seq, Ordering::SeqCst);
        Ok(())
    }

    /// Audit-only append (#8): the [`PgSink`] path when a broker
    /// confirmation runs without (or after a failed) full write-through.
    /// One INSERT-only transaction past the shared durable watermark.
    async fn append_audit(&self, events: &[AuditEvent]) -> Result<()> {
        let since = self.persisted_seq.load(Ordering::SeqCst);
        let pending: Vec<&AuditEvent> = events.iter().filter(|e| e.seq > since).collect();
        let Some(last) = pending.last() else {
            return Ok(());
        };
        let max_seq = last.seq;
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        for event in &pending {
            insert_audit_event(&tx, event).await?;
        }
        tx.commit().await.context("committing audit append")?;
        self.persisted_seq.fetch_max(max_seq, Ordering::SeqCst);
        Ok(())
    }

    /// `LogTestMessage`-style registration probe (#8): prove the write path
    /// through the append-only trigger with a real INSERT, then roll it
    /// back so the probe never occupies a sequence number.
    async fn probe_audit(&self) -> Result<()> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        tx.execute(
            "INSERT INTO audit_events (seq, at, actor, action, detail, app_id) \
             VALUES ($1, $2, 'audit-broker', 'audit.sink_probe', \
                     'registration self-test (rolled back)', NULL)",
            &[&i64::MAX, &(crate::state::now_unix() as i64)],
        )
        .await
        .context("audit probe INSERT")?;
        tx.rollback().await.context("audit probe rollback")?;
        Ok(())
    }
}

/// One audit row, INSERT-only, idempotent by seq. Shared by the
/// write-through batch and the audit-only [`PgSink`] append so the two
/// paths can never drift in shape.
async fn insert_audit_event(
    client: &impl tokio_postgres::GenericClient,
    event: &AuditEvent,
) -> Result<()> {
    let sensitive = serde_json::to_value(&event.sensitive).context("sensitive map serializes")?;
    // The Boundary-style paired plaintext rides its own column: the control
    // DB is the tenant-scoped store (apps.record already carries the
    // prompt), so the doctor's own audit view survives a restart. Every
    // *serialized* surface still carries only the HMAC form (see
    // src/audit.rs module doc / decision 0004).
    let sensitive_pt =
        serde_json::to_value(&event.sensitive_pt).context("sensitive_pt map serializes")?;
    client
        .execute(
            "INSERT INTO audit_events \
               (seq, at, actor, action, detail, app_id, sensitive, sensitive_pt) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (seq) DO NOTHING",
            &[
                &(event.seq as i64),
                &(event.at as i64),
                &event.actor,
                &event.action,
                &event.detail,
                &event.app_id,
                &sensitive,
                &sensitive_pt,
            ],
        )
        .await
        .with_context(|| format!("appending audit event {}", event.seq))?;
    Ok(())
}

/// The control-DB audit sink (#8): wraps the #7 `audit_events` table as a
/// broker sink. Shares the durable watermark with the write-through path,
/// so when a write-through already carried this operation's events the
/// confirmation is free; when it failed, `append` retries audit-first.
pub struct PgSink {
    store: Arc<PgStore>,
}

impl PgSink {
    pub fn new(store: Arc<PgStore>) -> Self {
        Self { store }
    }
}

impl crate::audit::AuditSink for PgSink {
    fn name(&self) -> &'static str {
        "control-db"
    }
    fn durable(&self) -> bool {
        true
    }
    fn confirmed_seq(&self) -> u64 {
        self.store.persisted_seq.load(Ordering::SeqCst)
    }
    fn probe(&self) -> crate::audit::SinkFuture<'_> {
        Box::pin(self.store.probe_audit())
    }
    fn append<'a>(&'a self, events: &'a [AuditEvent]) -> crate::audit::SinkFuture<'a> {
        Box::pin(self.store.append_audit(events))
    }
}

/// Write the platform's pending changes through to Postgres: the named app
/// records, every operation row touched since the last write-through, all
/// audit events past the durable watermark, and — for lifecycle changes —
/// the state-history row, atomically with its app upsert.
///
/// No-op (Ok) when no store is attached. Snapshots under a short write
/// lock, writes with NO lock held (F4 discipline applies here too). On
/// failure the drained operation rows are re-marked dirty so a later
/// write-through retries them; the caller decides whether the failure is
/// fatal (stage transitions: yes — revert and 503).
pub async fn write_through(
    platform: &SharedPlatform,
    app_ids: &[&str],
    transition: Option<StageTransition>,
) -> Result<()> {
    let (store, batch) = {
        let mut plat = platform.write().unwrap();
        let Some(store) = plat.store.clone() else {
            return Ok(());
        };
        let apps: Vec<AppRecord> = app_ids
            .iter()
            .filter_map(|id| plat.apps.get(*id).cloned())
            .collect();
        for app in &apps {
            if let Some(allocation) = app.allocation.as_ref() {
                allocation
                    .validate_cleanup_state()
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("app {} has contradictory cleanup state", app.id))?;
            }
        }
        let operations = plat.take_dirty_operations();
        let workspaces = app_ids
            .iter()
            .filter_map(|id| plat.workspaces.get(*id).cloned())
            .collect();
        let since = store.persisted_seq.load(Ordering::SeqCst);
        let audit: Vec<AuditEvent> = plat
            .audit
            .events()
            .iter()
            .filter(|e| e.seq > since)
            .cloned()
            .collect();
        let next_id = plat.next_id_counter();
        (
            store,
            PersistBatch {
                apps,
                workspaces,
                transition,
                operations,
                audit,
                next_id,
            },
        )
    };
    let result = store.persist(&batch).await;
    if result.is_err() {
        // Don't lose the drained rows — re-mark them dirty so the next
        // successful write-through retries them.
        platform
            .write()
            .unwrap()
            .remark_dirty_operations(&batch.operations);
    }
    result
}

/// Like [`write_through`], but non-fatal: a durability-degrading failure is
/// logged loudly and the touched rows retry on the next write-through.
/// Used only by best-effort operations (see the classification in
/// src/audit.rs); load-bearing operations go through `api::settle_durable`,
/// which applies the #8 broker invariant on top of this write.
pub async fn write_through_or_warn(
    platform: &SharedPlatform,
    app_ids: &[&str],
    transition: Option<StageTransition>,
) {
    if let Err(e) = write_through(platform, app_ids, transition).await {
        tracing::warn!("control DB write-through failed (durability degraded): {e:#}");
    }
}
