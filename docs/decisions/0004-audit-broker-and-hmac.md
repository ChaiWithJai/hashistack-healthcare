# Decision 0004 — Audit broker invariant and the salted-HMAC plaintext boundary

Status: decided (this link) · Related: issue #8,
[hashicorp-steering §2](../hashicorp-steering.md) (Vault `audit/` broker,
`IsFallback()`, `LogTestMessage`, `salt.GetIdentifiedHMAC`), issue #7
(control DB precursor), decision 0003 (evidence over claims).

## The rule

**No durable audit write, no operation.** When any durable sink is
configured, a load-bearing operation that cannot get ≥1 durable-sink
confirmation of its audit events fails with 503 audit-unavailable and its
state change reverts. In dev mode (no `AUDIT_FILE`, no `CONTROL_DB_URL`)
the in-memory fallback alone suffices and behavior is byte-identical to the
pre-#8 demo — the invariant arms itself exactly when durability exists to
enforce.

## Sink policy

| Sink | Configured by | Durable | Probe at registration |
|---|---|---|---|
| memory (`AuditLog`) | always — the fallback (`IsFallback`) | only in dev mode | trivial |
| file (`FileSink`) | `AUDIT_FILE` (JSONL, fsync per append) | yes | writes a probe line, fsyncs, reads it back |
| control-db (`PgSink`) | `CONTROL_DB_URL` (#7 `audit_events` table) | yes | INSERT through the append-only trigger, rolled back |

A sink that fails its probe is rejected loudly at boot (the boot fails).
Each durable sink keeps its own confirmed-seq watermark and retries the gap
on the next confirmation, so one slow/broken sink never loses events it
missed while the other carried the operation. Every sink failure lands an
`audit.sink_failed` event in the memory fallback — the failure itself is on
the record and replays into durable sinks once they recover.

## Operation classification

Load-bearing (durable confirm or revert + 503): scaffold settle
(`POST /api/apps`), applied iterate, gate fix, review, promote, rollback,
export (bundle withheld). Promote/rollback additionally keep #7's stricter
rule: the control-DB stage transition itself must commit, regardless of
other sinks. Best-effort: all reads, and `restore` (sandbox-only rebuild
from already-durable inputs). The classification is normative in the
`src/audit.rs` module doc; `tests/audit_broker_contract.rs` holds the
issue's bar (kill the sink → the next promotion 503s, app stays sandboxed,
`audit.sink_failed` in memory).

## The plaintext boundary (why some surfaces keep the doctor's words)

Doctor-authored free text — the describe prompt (`app.created`) and iterate
instructions (`app.iterated`) — is salted-HMAC'd (`hmac-sha256:<hex>`,
per-boot key, `AUDIT_HMAC_KEY` for cross-restart correlation) in the
event's `sensitive` map. `agent.attempt` events carry only op ids, tiers,
and machine-generated verdict reasons, so they need no envelope.

- **Platform-wide artifacts carry only the HMAC form**: `/api/audit/export`,
  the `AUDIT_FILE` archive, any future cross-tenant surface. A security
  reviewer can search and correlate ("this prompt appears in 3 events")
  without the platform export disclosing clinical narrative.
- **Tenant-scoped surfaces keep plaintext**: `/api/apps/:id/audit` (the
  doctor reading their own words) and the ejected COMPLIANCE.md (the
  doctor's own record, leaving WITH the doctor — HMAC'ing it would hold
  their own history hostage, against the eject bar in docs/GOAL.md).
- **The control DB stores the Boundary-style pt/HMAC pair** (`sensitive` +
  `sensitive_pt` columns). This is deliberate, not a leak: the control DB
  already stores the prompt in plaintext inside `apps.record` — it IS the
  tenant-scoped store. The pairing is what lets the tenant view survive a
  restart. The rule is about *surfaces*, not ciphertext-at-rest; real
  at-rest envelope encryption (Boundary ct/pt with `key_id`) is the #10
  identity/tenancy follow-on, not smuggled in here.

## Honest edges

- Object-storage archive sink and hipaa-core runtime-event ingestion from
  tenant apps (#5) are named in issue #8 but not built in this link; the
  `AuditSink` trait is the extension point and the issue stays open for
  them.
- The `AUDIT_FILE` archive appends one stream per boot (probe line
  timestamps each); with the control DB attached the watermark reconciles
  and streams stay gap- and duplicate-free across restarts. File-only
  restarts start a fresh numbered stream — readers dedupe by (boot, seq).
- Export digests for tamper-evidence beyond monotonic seq (hash chain) are
  not yet emitted; seq monotonicity is asserted in the pressure test.
