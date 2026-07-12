# Clinician Platform

Doctors describe small practice tools in ordinary language. The platform
builds a synthetic sandbox, explains what blocks release, and exports a Rust
application that the practice can run and change.

The gate between the sandbox and patient data is the central product decision.
An app cannot reach patient data while a required check is failing. A labeled
stub can publish only to the synthetic demo pool.

This repository implements the workflow from
[RFC 0001](docs/rfc/0001-clinician-platform.md):

```
describe -> generate -> preview -> iterate -> gate -> deploy -> operate -> audit
```

with the [design wireframes](docs/design/) (storyboards 1a builder / 1b pipeline /
1c clinical chart / 1d architecture) served as one wired doctor UI at `/`.

It is also still the proof repo it started as: the product decision note in
[docs/product-use-case.md](docs/product-use-case.md) names the managed default
(Lovable/Supabase glue — right for Tier 1–2 tools) and the Rust boundary (the
gate engine and append-only audit pipeline, where a wrong answer is a
reportable incident).

## What is proved

The integrated local tree has passed:

| Proof | Result |
|---|---:|
| Rust platform tests | 90 passed |
| Simulated pressure test | 89 passed |
| Nomad, Vault, and Postgres pressure test | 130 passed |
| User workflow checks | 458 of 458 passed |
| Built artifact checks | 432 of 432 passed |
| Runnable app packs | 17 |

The infrastructure proof schedules a generated Rust app on a Nomad client,
starts its container, publishes its port, and receives HTTP 200 from its health
route. It also proves Vault database credentials, Postgres recovery after a
signal 9 kill, and rollback. See
[the infrastructure proof](docs/evals/local-infrastructure-proof-2026-07-12.md).

These results prove the local learning product. They do not prove production
identity, workload identity, archive retention, or clinical fitness.

## Quick start

The portable end-user proof uses the same Compose bundle locally and on the
DigitalOcean/GCP single-host modules:

```bash
scripts/single-host-smoke.sh
# UI: http://localhost:3000 — bearer token: dev-token-osei
docker compose down                 # state and audit survive
docker compose down --volumes       # explicit destructive reset
```

This profile is for learning, customization, and owned export with synthetic
data. It is not a PHI-production or high-availability claim. See
[`terraform/single-host/`](terraform/single-host/) for provisioning.
The measured DigitalOcean procedure and topology decisions are in
[`docs/digitalocean-runbook.md`](docs/digitalocean-runbook.md).

For source development without containers:

```bash
cp env.example .env   # optional; runs without secrets
cargo run
# doctor UI:  http://127.0.0.1:3000/
curl http://127.0.0.1:3000/health
cargo test
```

Expected result:

- `/health` returns the control-plane status,
- 90 Rust tests pass, including the false pass guard,
- an app with a failing check receives HTTP 409 when promotion is requested,
- the workflow can be driven from the API by following
  [the runbook](docs/ops-runbook.md).

## Run the infrastructure proof

Docker Desktop can run the complete local infrastructure proof:

```bash
scripts/staging-docker-up.sh
```

The script starts Nomad, Vault, and Postgres and builds the local generated app
image. It prints the environment and control plane command. In a second
terminal, run:

```bash
scripts/pressure-test.sh http://127.0.0.1:39100
```

Set `NOMAD_REQUIRE_ALLOCATION=1` as printed by the setup script. The test then
requires a running allocation and successful health traffic. Stop the services
with `scripts/staging-docker-up.sh down`.

## What's here

| Piece | Path |
|---|---|
| Control plane API (16 routes, API-first) | `src/api.rs` |
| Pack registry — all 17 in-scope use cases across web, stream, and local profiles, each with runnable source, synthetic fixture, and quality contract | `src/packs.rs`, `packs/*/` |
| Gate engine — compliance checks as plugins ★ the product | `src/gates.rs` |
| Agent service — driver interface, rule-based Phase 0 driver | `src/agent.rs` |
| Routing ladder — verified escalation rules→local→frontier, pack-declared policy (#4, [decision 0001](docs/decisions/0001-agent-routing.md)) | `src/ladder.rs` |
| Deploy service — promote on green + co-sign, renders Nomad jobs | `src/deploy.rs`, `nomad/templates/` |
| Ejection service — owned bundle: docs from the record, derived pack (#11) | `src/eject.rs` |
| Audit pipeline — append-only, JSONL export | `src/audit.rs` |
| Practice Studio UI — clinician builder, release path, clinical view, and architecture, wired | `web/index.html` |
| 12-app outcome/customization/export profile | `docs/evals/sample-artifact-profiles.md` |
| Merge and release standard | `docs/process/merge-standard.md` |
| Doctor jobs from Anthony's podcast | `docs/research/anthony-doctor-jobs.md` |
| Infrastructure as code (Phase 1 substrate) | `terraform/prod/`, `packer/`, `vault/policies/` |
| Plan + design + steering | `docs/rfc/`, `docs/design/`, `docs/hashicorp-steering.md` |
| **The goal and the bar** | [`docs/GOAL.md`](docs/GOAL.md) |
| Use-case enablement investigation | [`docs/investigations/0001-enable-all-use-cases.md`](docs/investigations/0001-enable-all-use-cases.md) (#12) |
| Staging pressure test (`make staging`) | `scripts/pressure-test.sh` (#2) |
| Platform eval harness — job-to-be-done + artifact layers, scored per scenario | `evals/`, `scripts/evals.sh`, baseline in [`docs/evals/scorecard.md`](docs/evals/scorecard.md) |
| Local and DigitalOcean proof | [`docs/evals/local-infrastructure-proof-2026-07-12.md`](docs/evals/local-infrastructure-proof-2026-07-12.md), [`docs/evals/digitalocean-production-proof-2026-07-12.md`](docs/evals/digitalocean-production-proof-2026-07-12.md) |
| Cross-pack artifact evidence — actual job, ownership, safety/honesty, accessibility, docs | `packs/*/artifact-quality.json`, [`docs/evals/scorecard.md`](docs/evals/scorecard.md) |
| Journey profiler — one clinician journey run end-to-end: timed, audit-cross-referenced, ejected app driven | `scripts/journey.sh`, narrative in [`docs/evals/journey/journey.md`](docs/evals/journey/journey.md) |

The local product still uses development identity and a deterministic rules
driver by default. Production identity, signed attestations, enforcing workload
identity, and archive retention remain open. Current proof and limitations are
indexed in [docs/evidence-index.md](docs/evidence-index.md).

## The workflow, from curl

```bash
# describe → generate: sandbox pool, synthetic data only
curl -s -X POST localhost:3000/api/apps -H 'content-type: application/json' \
  -d '{"prompt":"post-op tracker for knee replacements","pack":"post-op-monitor","name":"post-op tracker"}'

# gate: preflight comes back 5/6 — auto-logoff not wired
curl -s localhost:3000/api/apps/post-op-tracker/gate

# promote while failing → 409, deploy locked, error names the check
# fix it for me → promote with a co-signature → prod pool allocation
curl -s -X POST localhost:3000/api/apps/post-op-tracker/gate/auto-logoff/fix -H 'content-type: application/json' -d '{}'
curl -s -X POST localhost:3000/api/apps/post-op-tracker/promote \
  -H 'content-type: application/json' -d '{"cosigner":"Dr. A. Osei"}'

# eject: the app as an owned bundle — README/runbook/compliance record
# generated from the doctor's record, deploy manifests for four targets,
# and a pack.hcl that makes their app their own re-importable template
curl -s localhost:3000/api/apps/post-op-tracker/export

# audit: the whole story, append-only
curl -s localhost:3000/api/audit/export
```

## Design authority

The Tao of HashiCorp, applied literally — workflows not technologies,
simple/modular/composable, immutability, codification, APIs first, pragmatism.
Extension happens through three plugin points (packs, drivers, gates), never
through forks of the core. We read the nomad/vault/packer/waypoint/boundary
trees to steer the details: [docs/hashicorp-steering.md](docs/hashicorp-steering.md).

## Test

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Proof

`make proof` prints the reviewer checklist; evidence rows live in
[docs/evidence-index.md](docs/evidence-index.md).

## Review and support

- Setup friction: open an issue with the `setup-friction` template.
- Lab bug: open an issue with the `lab-bug` template.
- Proof review: open an issue with the `proof-review` template or a pull request.

Reviewers should check the product decision before judging code style.
