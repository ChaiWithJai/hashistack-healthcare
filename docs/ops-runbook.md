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

## Troubleshooting
- If Rust is missing, install stable Rust and rerun CI commands, or `docker compose up --build`.
- If `nomad agent -dev` dies with `failed to detect memset: open
  /sys/fs/cgroup/cpuset/cpuset.mems` you are in a cgroup-v1 container without
  the cpuset controller mounted; as root:
  `mount -t cgroup -o cpuset cpuset /sys/fs/cgroup/cpuset` and rerun
  `scripts/staging-up.sh`. Stock GitHub runners (cgroup v2) are unaffected.
- If the service fails to bind, check whether port 3000 is already in use (or set `APP_BIND`).
- If tests fail, record the failure and the smallest fix in the evidence index.
- Platform state is in-memory in Phase 0: a restart clears apps and audit
  events by design. The Postgres control DB (with the Boundary-style
  `app_valid_state` transition table) is the Phase 1 replacement — see
  docs/hashicorp-steering.md §5.
