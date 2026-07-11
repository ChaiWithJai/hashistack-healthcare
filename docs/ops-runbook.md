# Ops Runbook

## Start
```bash
cargo run
# doctor UI: http://127.0.0.1:3000/
# override the bind address with APP_BIND=0.0.0.0:3000
```

## Smoke Check
```bash
curl -s http://127.0.0.1:3000/health
# {"status":"ok","service":"clinician-platform-control-plane"}

curl -s http://127.0.0.1:3000/api/packs | head -c 200
# five signed packs
```

## Drive the whole workflow from curl (the UI has no privileges you don't)
```bash
# describe → generate (sandbox, synthetic data)
curl -s -X POST localhost:3000/api/apps -H 'content-type: application/json' \
  -d '{"prompt":"post-op tracker for knee replacements","pack":"post-op-monitor","name":"post-op tracker"}'

# preflight — expect 5/6, auto-logoff failing
curl -s localhost:3000/api/apps/post-op-tracker/gate

# promotion is locked while a check fails (409)
curl -s -X POST localhost:3000/api/apps/post-op-tracker/promote \
  -H 'content-type: application/json' -d '{"cosigner":"Dr. A. Osei"}'

# fix it for me, then promote with a co-signature
curl -s -X POST localhost:3000/api/apps/post-op-tracker/gate/auto-logoff/fix -d '{}' -H 'content-type: application/json'
curl -s -X POST localhost:3000/api/apps/post-op-tracker/promote \
  -H 'content-type: application/json' -d '{"cosigner":"Dr. A. Osei"}'

# audit export (append-only, JSONL)
curl -s localhost:3000/api/audit/export
```

## Staging: the real HashiStack, virtually spinnable (#2)

One command boots a real Vault dev server (transit enabled), a real Nomad dev
agent, and the control plane wired to both — one machine, no cloud account:

```bash
scripts/staging-up.sh
# downloads pinned, checksum-verified nomad + vault into .staging/bin,
# boots vault (:8200), nomad (:4646), control plane (:39100),
# logs in .staging/logs/, pids in .staging/run/

# drive the whole workflow with real-infrastructure assertions:
NOMAD_ADDR=http://127.0.0.1:4646 VAULT_ADDR=http://127.0.0.1:8200 \
  scripts/pressure-test.sh http://127.0.0.1:39100

scripts/staging-up.sh down   # tear it all down
```

What changes when `NOMAD_ADDR` / `VAULT_ADDR`+`VAULT_TOKEN` are set on the
control plane (and only then — unset, behavior is exactly the simulation):

- promote renders the Nomad job and really submits it (`/v1/jobs/parse` →
  `/v1/jobs`); the returned evaluation id lands on the allocation as
  `nomad_eval_id`, and the tenant namespace is upserted first.
- promote proves the tenant's encryption keys against Vault: transit key
  `tenant-<tenant>` is created if missing and an encrypt/decrypt round-trip
  must return the probe intact (`vault_transit_key` on the allocation).
- rollback stops (not purges) the real job; if Nomad refuses, the rollback
  is refused too — the record never claims the sandbox while a job runs.
- a failed submission reverts the whole promotion (HTTP 502), so the record
  never claims "live" when real infrastructure said no.
- the audit stream gains `nomad.job_submitted`, `vault.transit_verified`,
  and `nomad.job_stopped` events as evidence.

Verify from the Nomad side directly:

```bash
export NOMAD_ADDR=http://127.0.0.1:4646
.staging/bin/nomad job status -namespace '*'
# promoted jobs show pending (placement is virtual), rolled-back jobs show
# dead (stopped)
curl -s "$NOMAD_ADDR/v1/job/<app-id>?namespace=tenant-meridian" | python3 -m json.tool
```

Staging caveats (deliberate, documented):

- The dev agent has no Vault workload identity, so the job's `vault` stanza
  is stripped at submission; the control plane proves the Vault side itself
  via the transit probe. Cloud staging keeps the stanza.
- The job pins `datacenters = ["nyc3"]` and `meta.role = "prod"`, which the
  single dev agent does not satisfy — registration and scheduling are real,
  placement stays pending. The pressure test asserts registration and stop,
  not a running container.
- CI: `staging-hashistack` (.github/workflows/staging.yml) runs this nightly
  and on demand; every PR still runs the simulated pressure test (ci.yml).

## Control DB (#7): Postgres with a database-enforced lifecycle

`CONTROL_DB_URL` unset (the default) → the platform is in-memory, exactly
as before. Set it and the control plane becomes restart-survivable:

```bash
# staging-up.sh boots a portable postgres on 127.0.0.1:5433 and exports this:
CONTROL_DB_URL=postgres://staging@127.0.0.1:5433/control cargo run
# boot log: "control DB attached — restored N apps, N operations, N audit events"
```

What the database enforces (migrations/0001_init.sql, applied idempotently
at every boot — Boundary's pattern, steering §5):

- `app_valid_state(prior_state, current_state)` — the legal lifecycle
  transitions, seeded from the SAME Rust const the in-memory checks use
  (`state::VALID_STAGE_TRANSITIONS`; tests/store_contract.rs pins them to
  each other). A trigger on `apps` rejects any stage UPDATE not in the
  table — application bugs cannot half-promote an app.
- `app_state_history` — append-only (UPDATE/DELETE rejected by trigger; no
  grants beyond INSERT+SELECT), one row per stage change with a composite
  FK into `app_valid_state`, so an illegal recorded transition is
  unrepresentable.
- `operations` — Waypoint upsert-first rows (§4): `running` is written
  before any driver runs, so a `kill -9` leaves the interrupted operation
  visible after restart, never invisible.
- `audit_events` — append-only by trigger, same numbering as the in-memory
  stream.

Semantics when attached:

- In-memory state stays the read path; Postgres is durability. Every
  mutation writes through AFTER its platform lock is released.
- **A stage transition the DB did not confirm did not happen**: if the
  write-through for promote/rollback fails, the in-memory record reverts
  and the API returns 503 (`app.promotion_reverted` /
  `app.rollback_reverted` land in the audit stream). #8 generalizes this
  into the audit broker invariant below.
- Other write failures degrade durability, log a loud warning, and retry
  on the next write-through; whether the *operation* stands is then the
  audit broker's call (below).
- Boot loads everything back: apps, operations, audit events, and the
  id-minting counter (`control_meta`), so restored ids never collide.

Verified virtually: `scripts/pressure-test.sh` with `CONTROL_DB_URL` set
kills the control plane with SIGKILL right after promote, reboots it on the
same database, and asserts the app is still live with its allocation,
attestation, audit history, and operation rows intact. CI runs this nightly
in `staging-hashistack` against a `postgres:16` service container.

Inspect the evidence directly:

```bash
.staging/postgres/bin/psql "$CONTROL_DB_URL" -c \
  'SELECT app_id, prior_state, current_state, at FROM app_state_history ORDER BY id'
# and watch the schema refuse tampering:
.staging/postgres/bin/psql "$CONTROL_DB_URL" -c 'DELETE FROM audit_events'
# ERROR:  audit_events is append-only: DELETE rejected
.staging/postgres/bin/psql "$CONTROL_DB_URL" -c \
  "INSERT INTO app_state_history (app_id, prior_state, current_state, at) \
   VALUES ('x', 'live', 'live', 0)"
# ERROR:  violates foreign key constraint (no such pair in app_valid_state)
```

Honest edge: with only two stages, both cross-transitions are legal, so the
stage trigger cannot fire on today's reachable states — it (plus the FK and
the CHECK) is the enforcement that keeps the table authoritative the moment
a third stage (`gate_pending`, per issue #7's target shape) lands.

## Audit broker (#8): no durable audit write, no operation

Vault's `audit/` broker pattern, applied (steering §2). Sinks register
behind a broker; each runs a `LogTestMessage`-style probe at registration
and a failing sink **fails the boot**:

- **memory** — always present, the fallback sink: events (including the
  record of any other sink's failure) are never lost. Counts as durable
  only in dev mode (nothing else configured), where behavior is
  byte-identical to the plain demo.
- **file** — `AUDIT_FILE=/path/audit.jsonl`: one JSON event per line,
  fsync on every append, probe line per boot stream. `staging-up.sh`
  attaches it by default at `.staging/logs/audit.jsonl`.
- **control-db** — attached with `CONTROL_DB_URL` (#7 `audit_events`).

With any durable sink attached, **load-bearing operations** (create,
applied iterate, gate fix, review, promote, rollback, export) fail with
503 audit-unavailable — and their state change reverts — unless ≥1 durable
sink confirms the audit events. Reads and `restore` stay best-effort. Full
classification: `src/audit.rs` module doc; rationale: decision 0004.

Sensitive values (the doctor's prompt and iterate instructions) are salted
HMACs on every platform-wide surface (`/api/audit/export`, the `AUDIT_FILE`
archive): `"prompt":"hmac-sha256:<hex>"` — searchable, correlatable, not
disclosable. The doctor's own app-scoped view (`/api/apps/:id/audit`) and
their ejected COMPLIANCE.md keep their plaintext. Set `AUDIT_HMAC_KEY` to
correlate across restarts; unset, the salt is random per boot.

Watch the invariant hold (control-db as the only durable sink; an open
file handle is hard to break from outside, which is exactly why the
contract test injects a killable sink):

```bash
CONTROL_DB_URL=postgres://staging@127.0.0.1:5433/control cargo run &
# … create + fix an app, then kill the sink and try to promote:
.staging/postgres/bin/pg_ctl -D .staging/pgdata stop -m immediate
curl -s -X POST localhost:3000/api/apps/<id>/promote \
  -H 'content-type: application/json' -d '{"cosigner":"Dr. A. Osei"}'
# → 503 {"error":"promotion reverted — control DB refused or missed …"}
# the app is still sandboxed; audit.sink_failed / app.promotion_reverted
# are on the in-memory fallback record and replay once the sink returns
```

Verified automatically: `tests/audit_broker_contract.rs` (kill-the-sink →
503 + sandboxed + `audit.sink_failed`; HMAC boundary end-to-end over a real
file sink) and the pressure test's `audit broker (#8)` section, which runs
with the file sink attached on every self-booted run and additionally
asserts the promote audit row + HMAC'd prompt inside Postgres when
`CONTROL_DB_URL` is set.

## Eject an app (#11): an owned, documented, extendable bundle

`GET /api/apps/:id/export` returns a JSON file-map — README, runbook, and
compliance record generated from the doctor's own record (prompt, addenda,
gate report, attestation, audit trail), deploy manifests for
Nomad/Render/Fly/Kamal, and a `pack.hcl` that turns the app into their own
re-importable template. Works for sandbox apps too, with the compliance
record marked `draft — not released` and no attestation.

```bash
# unpack the bundle into ./post-op-tracker with stock python3 (no extra deps)
mkdir -p post-op-tracker && cd post-op-tracker && \
curl -s localhost:3000/api/apps/post-op-tracker/export | \
python3 -c 'import json,sys,pathlib; [(lambda q: (q.parent.mkdir(parents=True,exist_ok=True), q.write_text(c)))(pathlib.Path(p)) for p,c in json.load(sys.stdin)["files"].items()]'

# then follow the bundle's own docs — that is the point:
cat docs/RUNBOOK.md
```

## Agent routing ladder (#4, [decision 0001](decisions/0001-agent-routing.md))

Every agent action (scaffold, iterate, fix) is a Waypoint-style operation —
upserted `running` before any driver runs, settled after each attempt —
climbing the verified ladder rules → local → frontier. A deterministic
verifier (gate preflight on a cloned record) judges every tier's output;
routing emerges from verdicts, not prediction. Pack `routing` policy in
pack.hcl picks each action's first tier and consents to frontier escalation.

```bash
# both unset (default): rules-only ladder, exactly today's behavior
# local tier: any in-VPC OpenAI-compatible endpoint (vLLM, llama.cpp, LM Studio)
LOCAL_MODEL_URL=http://127.0.0.1:8080 \
FRONTIER_MODEL_URL=http://127.0.0.1:8090 \
  cargo run

# the routing record for one app: attempt history, tiers, verdicts
curl -s localhost:3000/api/apps/post-op-tracker/operations
# every decision is also in the audit stream:
#   agent.routed  — "per pack ... routing policy: iterate→local"
#   agent.attempt — "op op-1234 iterate v2 tier=local verdict=accepted → applied"
```

Plain `http://` only (refusing TLS refuses off-VPC by construction); debug
and test builds additionally refuse non-loopback endpoints (decision 0002).
Offline or misbehaving model tiers cost rejected attempts, never a broken
app — the rules floor still lands the doctor's edit. Staging model serving
is stubbed at `scripts/staging-up.sh --models` (decision 0002).

F4 (resolved in #7): model calls run on the blocking pool with NO platform
lock held — a slow tier can never stall unrelated requests (asserted by
`slow_local_tier_does_not_block_a_concurrent_unrelated_request`). If the
record changes while a tier thinks, the verified edit is not applied; the
operation settles `concurrent-edit` and the API returns 409 — retry the
instruction. Setting `LOCAL_MODEL_URL` in a shared environment is safe now.

## Troubleshooting
- If Rust is missing, install stable Rust and rerun CI commands, or `docker compose up --build`.
- If `nomad agent -dev` dies with `failed to detect memset: open
  /sys/fs/cgroup/cpuset/cpuset.mems` you are in a cgroup-v1 container without
  the cpuset controller mounted; as root:
  `mount -t cgroup -o cpuset cpuset /sys/fs/cgroup/cpuset` and rerun
  `scripts/staging-up.sh`. Stock GitHub runners (cgroup v2) are unaffected.
- If the service fails to bind, check whether port 3000 is already in use (or set `APP_BIND`).
- If tests fail, record the failure and the smallest fix in the evidence index.
- Without `CONTROL_DB_URL`, platform state is in-memory and a restart clears
  apps and audit events by design. Set `CONTROL_DB_URL` (see "Control DB"
  above) for the Boundary-style durable state machine (#7).
- If boot fails with "connecting to control DB", the URL points at a
  postgres that isn't up — `scripts/staging-up.sh` boots one on :5433, or
  unset `CONTROL_DB_URL` to run in-memory.
