# Clinician Platform — Phase 0 control plane

Lovable for clinicians, on HashiStack + DigitalOcean. Doctors describe practice
tools in natural language and receive running, HIPAA-scaffolded applications —
with a **gate** between the sandbox and real patients. The gate step is the
product: consumer builders go describe → deploy with nothing in between.

This repo is the Phase 0 slice of [RFC 0001](docs/rfc/0001-clinician-platform.md):
a Rust control plane implementing the full workflow contract

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

## Quickstart

```bash
cp env.example .env   # optional; runs without secrets
cargo run
# doctor UI:  http://127.0.0.1:3000/
curl http://127.0.0.1:3000/health
cargo test
```

Expected proof:

- `/health` returns the control-plane status,
- 18 tests pass, including the false-pass guard: an app with a failing
  compliance check **cannot** be promoted (409, error names the check),
- the whole workflow is drivable from curl ([docs/ops-runbook.md](docs/ops-runbook.md)) —
  the UI holds no privileges the API doesn't offer.

## Docker Fallback

```bash
docker compose up --build
```

## What's here

| Piece | Path |
|---|---|
| Control plane API (16 routes, API-first) | `src/api.rs` |
| Pack registry — signed HCL use-case packs | `src/packs.rs`, `packs/*/pack.hcl` |
| Gate engine — compliance checks as plugins ★ the product | `src/gates.rs` |
| Agent service — driver interface, rule-based Phase 0 driver | `src/agent.rs` |
| Routing ladder — verified escalation rules→local→frontier, pack-declared policy (#4, [decision 0001](docs/decisions/0001-agent-routing.md)) | `src/ladder.rs` |
| Deploy service — promote on green + co-sign, renders Nomad jobs | `src/deploy.rs`, `nomad/templates/` |
| Ejection service — owned bundle: docs from the record, derived pack (#11) | `src/eject.rs` |
| Audit pipeline — append-only, JSONL export | `src/audit.rs` |
| Doctor UI — wireframes 1a/1b/1c/1d, wired | `web/index.html` |
| Infrastructure as code (Phase 1 substrate) | `terraform/prod/`, `packer/`, `vault/policies/` |
| Plan + design + steering | `docs/rfc/`, `docs/design/`, `docs/hashicorp-steering.md` |
| **The goal and the bar** | [`docs/GOAL.md`](docs/GOAL.md) |
| Use-case enablement investigation | [`docs/investigations/0001-enable-all-use-cases.md`](docs/investigations/0001-enable-all-use-cases.md) (#12) |
| Staging pressure test (`make staging`) | `scripts/pressure-test.sh` (#2) |

**Honest label:** what runs today is a skinned UI over a simulated control
plane — the workflow contract is real and tested; the platform underneath it
is tracked in issues #2–#11, referenced as `TODO(#n)` at each simulation
point in the source.

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

## Review And Support

- Setup friction: open an issue with the `setup-friction` template.
- Lab bug: open an issue with the `lab-bug` template.
- Proof review: open an issue with the `proof-review` template or a pull request.

Reviewers should check the product decision before judging code style.
