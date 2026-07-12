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

These examples send no `Authorization` header, which works in dev because
of the audited dr-osei fallback (see "Identity & tenancy" below). Add
`-H 'authorization: Bearer dev-token-osei'` and they work against a strict
(staging) instance too.

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
  and `nomad.job_stopped` events as evidence — and, with the control DB
  attached, the #9 events below.

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

## Vault (#9): dynamic DB creds, per-tenant policies, audit device

With `VAULT_ADDR`+`VAULT_TOKEN` AND `CONTROL_DB_URL` set (staging-up.sh sets
all three), the RFC's "compliance spine" claims become exercised facts:

- **staging-up.sh wires Vault's database engine to the staging Postgres**:
  connection `database/config/staging-postgres` (the control-DB superuser as
  root credential) and role `tenant-app` — a creation template granting
  login + CRUD on the tenant/control DB, `default_ttl=1h`, `max_ttl=2h`.
  Host auth is scram-sha-256 (trust is tightened at boot), so password
  checks below are real, not vacuous.
- **promote issues per-allocation dynamic creds** (`database/creds/
  tenant-app`) and VERIFIES them before recording anything: the control
  plane opens a one-shot connection AS the issued user and runs `SELECT 1`.
  The allocation's placeholder `credentials` string is replaced by the real
  lease (`vault_lease_id`, `vault_db_username`, `vault_lease_ttl_secs`);
  audit gains `vault.db_creds_issued`. The password is quarantined in
  `hashi::DbLease` — never on the allocation, the audit stream, or any
  durable surface (a cargo test greps an audit export to hold this).
- **rollback revokes the lease and PROVES it** (`sys/leases/revoke`, only
  after Nomad accepted the stop — #2's refusal semantics): the issued user
  must fail to authenticate and must be gone from `pg_roles` (the engine's
  revocation drops the role), or the rollback errors. Audit gains
  `vault.lease_revoked`. Proof shape, honestly: the control plane retains
  no password (secrets never persist; a kill -9 sits between promote and
  rollback in the pressure test), so its proof is login-failure +
  role-absence; the pressure test additionally holds a **sibling lease's
  password** end-to-end and asserts the literal authenticate-then-fail.
- **tenant first-promote mounts the tenant's ACL policy**: the rendered
  `vault/policies/tenant-app.hcl` lands at `sys/policies/acl/tenant-
  <tenant>`, naming the exact transit + database paths; audit gains
  `vault.policy_mounted` and the pressure test reads the policy back.
- **Vault's file audit device is on from first boot**
  (`.staging/logs/vault-audit.log`) — the Vault audit log is itself the
  HIPAA technical-safeguard artifact (RFC 0001); staging refuses to come up
  without it, and the pressure test asserts it carries the transit request
  path after a promote.
- **rotate-proof**: the pressure test encrypts, rotates the tenant transit
  key (`transit/keys/<key>/rotate`), and decrypts the pre-rotation
  ciphertext intact — key versioning means rotation never strands data.

Inspect the evidence directly:

```bash
export VAULT_ADDR=http://127.0.0.1:8200 VAULT_TOKEN=staging-root
.staging/bin/vault policy read tenant-meridian        # the mounted policy
.staging/bin/vault read database/creds/tenant-app     # mint a lease yourself
.staging/bin/vault list sys/leases/lookup/database/creds/tenant-app
grep transit/encrypt .staging/logs/vault-audit.log | tail -3
```

Honest edges (deliberate, documented):

- **Dev-mode tokens are root**: the per-tenant policy exists, names the
  live paths, and reads back — but the control plane still authenticates
  with the root token, so the policy is not yet the enforcing credential.
  Enforcement-by-token (per-allocation tokens bound to the tenant policy,
  Vault workload identity on the Nomad client) is the Phase 1 cloud item.
- **One shared database role** (`tenant-app`) serves all tenants in
  staging; per-tenant DB roles land with Phase 1. The rendered Nomad job
  and the policy template say exactly this.
- Creds are issued per *allocation as recorded*, not injected into a
  running container (placement stays virtual in staging, see #2 caveats);
  the consul-template stanza in the rendered job is the cloud path.

## Control DB (#7): Postgres with a database-enforced lifecycle

`CONTROL_DB_URL` unset (the default) → the platform is in-memory, exactly
as before. Set it and the control plane becomes restart-survivable:

```bash
# staging-up.sh boots a portable postgres on 127.0.0.1:5433 (host auth
# scram-sha-256, fixed dev credential like VAULT_TOKEN) and exports this:
CONTROL_DB_URL=postgres://staging:staging-pg@127.0.0.1:5433/control cargo run
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
CONTROL_DB_URL=postgres://staging:staging-pg@127.0.0.1:5433/control cargo run &
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

## Identity & tenancy (#10): principals, roles, sessions

Every `/api` route resolves an authenticated principal; `/`, `/health`,
`/proof`, and the static UI stay open. Principals are declared in
`staging/identities.hcl` (parsed with hcl-rs like packs; an unknown role or
attribute, a duplicate token, or a blank field fails the boot loudly):

```hcl
identity "dr-osei" {
  name   = "Dr. A. Osei"
  role   = "clinician"   # clinician | staff — a closed set
  tenant = "meridian"
  token  = "dev-token-osei"
}
```

**Honest labeling — static tokens are the Phase 0 dev credential** (same
spirit as `VAULT_TOKEN=staging-root`). OIDC replaces the token *source*
(issuer-verified id_token → principal), **not the model**: principal id,
name, role, tenant, and every enforcement below survive that upgrade
unchanged. NPI-verified clinician identity (RFC open question 2) rides the
same seam. Never put a production credential in an identities file.

Two modes, both deliberate:

- **Dev (no `IDENTITIES_FILE`)** — the embedded copy of
  staging/identities.hcl applies. A request with NO Authorization header
  falls back to `dr-osei`, so the zero-config doctor UI keeps working —
  and the trail confesses it with one `auth.dev_fallback` event per boot.
  A *present but unknown* token is still 401 even in dev.
- **Strict (`IDENTITIES_FILE=staging/identities.hcl`)** — staging-up.sh
  sets this: missing or invalid tokens answer 401, no fallback.

What the resolved principal enforces:

- **Tenancy on every app-scoped route**: a cross-tenant id answers **404**
  exactly like a nonexistent one (existence is never disclosed), with an
  `auth.cross_tenant_denied` audit event on the owning tenant's app stream
  (the practice sees who knocked). Lists filter to the caller's tenant;
  app creation takes its tenant from the principal (a request naming any
  other tenant is 422). The #8 plaintext boundary is keyed to the
  requesting principal's tenant: the app-scoped audit view and the ejected
  COMPLIANCE.md render plaintext only inside the owning tenant.
- **Roles as one capability check** (src/identity.rs): clinicians do
  everything in their tenant; **staff cannot promote/co-sign or export the
  platform-wide audit** — 403 with `auth.role_denied` (role denial may be
  403: in-tenant existence is already known). Staff can build, iterate,
  fix, review, operate, export the app bundle, and roll back (withdrawing
  from use must never wait on the clinician).
- **The co-sign is cryptographic**: promotion requires an authenticated
  clinician of the app's tenant; the attestation records the principal id,
  their registered display name, and `report_digest` — sha256 over the
  frozen gate report's canonical JSON — plus the timestamp. The typed
  `cosigner` field survives only as a display-name check (must match the
  principal's registered name, or be omitted). Verify a record yourself:
  recompute sha256 over the attestation's embedded `report` JSON and
  compare (`gates::report_digest`; asserted in tests/identity_contract.rs).
- **Session idle — the platform honors its own auto-logoff gate**:
  `SESSION_IDLE_SECS` (staging default 900; off in dev) expires a token
  idle past the limit → 401 + `auth.session_expired`. Implementation is
  deliberately simple: in-memory last-seen per token; with static Phase 0
  tokens the 401 is the logoff boundary and the next request starts a
  fresh session (an OIDC credential makes the 401 terminal).

```bash
# strict staging instance:
TOKEN_OSEI="${STUDIO_TOKEN_OSEI:?set the Osei staging token}"
TOKEN_PARK="${STUDIO_TOKEN_PARK:?set the Park staging token}"
TOKEN_RIVERA="${STUDIO_TOKEN_RIVERA:?set the Rivera staging token}"
curl -s http://127.0.0.1:39100/api/apps                                   # 401
curl -s -H "authorization: Bearer ${TOKEN_OSEI}" \
  http://127.0.0.1:39100/api/apps                                         # 200, meridian only
curl -s -H "authorization: Bearer ${TOKEN_PARK}" \
  http://127.0.0.1:39100/api/apps                                         # 200, lakeside only
curl -s -X POST -H "authorization: Bearer ${TOKEN_RIVERA}" \
  http://127.0.0.1:39100/api/apps/<id>/promote -d '{}' \
  -H 'content-type: application/json'                                     # 403 (staff)
```

Honest edges (deliberate, documented):

- The doctor UI ships with no token store, so against a strict instance it
  answers 401 — the zero-config UI is the *dev* experience; the staging UI
  story lands with the OIDC login. Drive staging with curl + tokens.
- Operator access (staff debugging a tenant app) is **absent by design**,
  not forgotten: decision 0005 records the Boundary Target/Session shape
  (max duration, recording flag, `termination_reason`) it must take;
  today staff have no cross-tenant read at all.
- Audit actors that are platform services ("agent", "deploy",
  "gate-engine", "platform-reviewer") stay service names; every
  doctor-initiated action carries the real principal id.

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

## Recover a pending rollback

`allocation.cleanup_pending=true` means the platform durably accepted a
withdrawal but has not yet proved every external cleanup step. Do not edit,
review, restore, or promote that app. Those endpoints return 409 until the
withdrawal settles.

1. Read `GET /api/apps/:id/operate`. Desired state is `stopped`.
   `cleanup_workload_stopped=true` and `status_source=rollback-cleanup` mean
   Nomad's stop was already confirmed; otherwise the observed state is polled
   from Nomad.
2. Confirm `NOMAD_ADDR` is available when `nomad_eval_id` exists. Confirm
   `VAULT_ADDR`, `VAULT_TOKEN`, and the staging/control database URL are
   available when a Vault lease is recorded. Missing proof clients fail
   closed; never delete the handles by hand.
3. Retry `POST /api/apps/:id/rollback`. A confirmed stop is not repeated.
   Vault revocation and Postgres role absence are verified before the app can
   become `sandbox` and return to synthetic data.
4. A 502 means external cleanup remains retryable. A 503 means the control DB
   or durable audit sink did not confirm progress; restore that dependency
   before retrying. Inspect the app-scoped audit stream for
   `app.rollback_requested` and `app.rollback_cleanup_pending`.

Only bounded error codes are returned in the allocation record. Full backend
causes stay in protected service logs so tokens, URLs, or response bodies do
not become tenant-facing state.

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
