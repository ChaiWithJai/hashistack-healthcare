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

## Troubleshooting
- If Rust is missing, install stable Rust and rerun CI commands, or `docker compose up --build`.
- If the service fails to bind, check whether port 3000 is already in use (or set `APP_BIND`).
- If tests fail, record the failure and the smallest fix in the evidence index.
- Platform state is in-memory in Phase 0: a restart clears apps and audit
  events by design. The Postgres control DB (with the Boundary-style
  `app_valid_state` transition table) is the Phase 1 replacement — see
  docs/hashicorp-steering.md §5.
