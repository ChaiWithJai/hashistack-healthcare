# ADR 0006: Durable mutation and rollback-cleanup invariants

Status: proposed; acceptance requires the proof obligations below

## Decision

Mutations for one app are serialized until both the state change and its audit
record receive a durable verdict. Mutations for different apps may proceed in
parallel. This is the smallest concurrency boundary that prevents a later
successful edit from depending on an earlier edit that is subsequently
reverted when audit persistence fails.

Rollback is a small saga with two independently observable steps: stopping the
Nomad workload, then revoking and verifying the Vault lease. The durable
states are:

| State | Stage | Desired/observed | Allowed actions |
| --- | --- | --- | --- |
| Active | live | running / Nomad observation | normal app actions |
| Cleanup requested | live | stopped / Nomad observation | rollback retry, operate, audit |
| Workload stopped | live | stopped / stopped | rollback retry, operate, audit |
| Sandbox | sandbox | stopped / stopped | normal sandbox actions |

The cleanup-requested record is settled before Nomad is called. If the stop
succeeds but lease cleanup fails, `cleanup_workload_stopped` is settled while
the allocation remains `live` as an ownership record. Retrying skips the
already-confirmed stop, retries credential cleanup, and changes the app to
`sandbox` only after cleanup is verified. Missing clients for any persisted
Nomad, Vault, or database-verification handle fail closed.

## Why

Restoring a `live` and apparently running snapshot after Nomad has stopped is a
false operational claim. Conversely, deleting the allocation record after a
Vault failure loses the handles needed to revoke credentials. The explicit
intermediate state preserves truth and makes cleanup retryable without adding
a general workflow engine.

Per-app locks are process-local. They are sufficient for the documented
single-host MVP on local Docker and one DigitalOcean Droplet. Horizontal API
replicas require replacing this lock with a database advisory lock or an
equivalent per-app compare-and-set transaction before scale-out is supported.

## Proof obligations

- Stop failure never records the workload as stopped.
- Revoke or verification failure after stop produces cleanup-pending state.
- A retry does not stop the same confirmed-stopped workload again.
- A later same-app mutation waits for the earlier durable verdict.
- If the earlier mutation is reverted, the later mutation starts from the
  reverted state and can still commit successfully.
- `operate` reports cleanup-pending allocations as stopped, never simulated or
  healthy-running.
- External cleanup for one app does not hold the global state lock or block a
  mutation for a different app.
- Backend failure text and credentials never enter the persisted allocation or
  API response.

Async request cancellation remains outside this ADR's accepted claim until a
candidate-state guard or database transaction makes publish/compensation
cancellation-safe. The contract currently covers completed HTTP handlers and
explicit backend failures; this limitation is a merge gate for declaring the
general durability invariant complete.
