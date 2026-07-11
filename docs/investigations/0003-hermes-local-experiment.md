# Investigation 0003 — First real staging model tier + hermes-agent as an external client

Experiment `hermes-local`, run 2026-07-11 on branch `claude/experiment-hermes-local`
(based on `claude/issue-5-runnable-packs`). Two questions:

- **A.** Does the staging model tier become real — a local llama.cpp server
  behind `LOCAL_MODEL_URL`, driving the #4 escalation ladder — and what are
  the first live numbers for decision 0001's staging metrics?
- **B.** Can [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent)
  act as an external, unprivileged client of the control-plane API, and is it
  a candidate runtime for Wave-4 local packs (deid-local, note-extraction)?

Related: [decision 0001](../decisions/0001-agent-routing.md),
[decision 0002](../decisions/0002-inference-test-tiers.md),
[investigation 0002](0002-local-model-simplifiers.md) D4/D5.

## Executive summary

The machinery is proven; the small models are not — which is exactly what
decision 0002 predicted. `staging-up.sh --models` now really fetches pinned,
sha256-verified GGUF weights and serves them on `127.0.0.1:8081`; the full
staging stack (Vault + Nomad + control plane) ran against it and passed the
pressure test 49/49. Across 18 live ladder operations the **local tier's
accept rate was 0%** — every attempt was rejected by the deterministic
verifier and escalated cleanly to the rules floor, with every decision
reconstructable from the audit stream alone. hermes-agent installed and
confined cleanly, but with a ≤270M local model it **emitted zero tool calls
in two attempts** — the control plane never received a single request from
it. Sub-1B models can't drive an agent framework; they *can* (and did)
exercise our escalation path, which is the staging tier's actual job.

## Setup that actually ran

Environment note that shaped everything: this container's egress policy
blocks `huggingface.co`, GitHub release assets, and OCI blob CDNs, but allows
PyPI. So the preferred Liquid LFM2 GGUF (decision 0002) was unreachable, and
the pinned artifacts below are GGUF files published inside PyPI wheels —
sha256-verified at both hops (wheel hash, then assembled `.gguf` hash).

| Component | What ran | Provenance / pin |
|---|---|---|
| Server | `llama-cpp-python[server]==0.3.16` (CPU build from PyPI sdist) in `.staging/models/venv`, OpenAI-compatible `/v1/chat/completions` on `127.0.0.1:8081` | version-pinned; PyPI TLS |
| Model 1 (default) | SmolLM2-135M-**Instruct** Q4_1 (94 MB, Apache-2.0) | `llm-smollm2==0.1.2` wheel (Simon Willison), wheel sha256 `bcc81830…`, gguf sha256 `b179c952…` |
| Model 2 (weak rung) | gemma-3-270m Q4_K_M (242 MB, Gemma terms) — **base model, not -it**; the upstream GGUF was converted from `google/gemma-3-270m` | `gemma3-270m-q4-k-m-gguf-part1..4==1.0.0` wheels, per-wheel sha256 pins, assembled gguf sha256 `a5fd3b62…` |
| Stack | `staging-up.sh`: Vault 1.17.6 dev + Nomad 1.8.4 dev + control plane on `:39100`, `LOCAL_MODEL_URL=http://127.0.0.1:8081`, `MODEL_HTTP_TIMEOUT_SECS=60` | cgroup-v1 cpuset mount fix from the runbook was needed and worked |

Caveats, stated plainly: (1) the gguf checksums are pinned to what was
fetched and structurally verified (GGUF metadata: `gemma3` 270M / `llama`
SmolLM2 135M, correct tensor counts) — they could **not** be cross-checked
against vendor-published checksums because HF is unreachable from here; swap
in the LFM2 GGUF + official hash when the network allows. (2) GGUF metadata
confirms the gemma file is the base model; it was kept deliberately as the
"weak rung" fixture decision 0002 wants, and labeled as such.

Total experiment downloads ≈ 1.1 GB; disk high-water ≈ 2.8 GB (under budget).

## PART A — the ladder against a real local tier (first live data)

Method: `scripts/ladder-exercise.sh` (committed) creates a post-op-monitor
app and runs 8 iterate instructions of varying difficulty, then reads
`/api/apps/:id/operations` and the audit stream back. post-op-monitor
declares no `routing`, so platform defaults applied: scaffold→frontier
(unconfigured → resolved down to local), iterate→local.

### Per-instruction results

Verdicts identical for both models — every local attempt rejected, every
operation settled `success` on the rules floor. Latency is where they differ.

| Instruction | Tiers climbed | Verdict at local (reason) | local attempt s (SmolLM2 / gemma) | end-to-end ms (SmolLM2 / gemma) |
|---|---|---|---|---|
| *(scaffold from pack)* | local → rules | rejected (empty-scaffold) | 4 / 8 | — |
| add automatic logoff after 15 min idle | local → rules | rejected (empty-edit) | 4 / 2 | 3675 / 2378 |
| remind patients to log wound photos daily | local → rules | rejected (empty-edit) | 0 / 7 | 364 / 6901 |
| staff triage queue with role-based access | local → rules | rejected (empty-edit) | 4 / 6 | 3466 / 6591 |
| flag rising pain scores, escalate to surgeon | local → rules | rejected (empty-edit) | 1 / 7 | 1523 / 7454 |
| translate check-in form to Spanish | local → rules | rejected (empty-edit) | 3 / 7 | 3099 / 7031 |
| **remove the audit log** (adversarial) | local → rules | rejected (empty-edit) | 3 / 7 | 2628 / 6620 |
| dashboard chart of weekly pain trends | local → rules | rejected (empty-edit) | 4 / 0 | 3850 / 127 |
| patients export their data as a PDF | local → rules | rejected (empty-edit) | 3 / 7 | 3499 / 6917 |

### Per-tier summary (both runs)

| Tier | Attempts | Accepted | Accept rate | Reject reasons |
|---|---|---|---|---|
| local (SmolLM2-135M-Instruct Q4_1) | 9 | 0 | **0.0%** | empty-edit ×8, empty-scaffold ×1 |
| local (gemma-3-270m base Q4_K_M) | 9 | 0 | **0.0%** | empty-edit ×8, empty-scaffold ×1 |
| rules (floor) | 18 | 18 | 100% | — |

### What "empty-edit" actually was (raw-endpoint sub-classification)

The verifier's reason classes collapse several failures; sampling the raw
endpoint with the driver's exact prompt (4 instructions per config) splits
them:

| Config | Unparseable prose | Parseable JSON, semantically empty | Would-accept |
|---|---|---|---|
| SmolLM2, free-form (what the driver sends) | 4/4 | 0 | 0 |
| gemma base, free-form | 4/4 | 0 | 0 |
| SmolLM2, `response_format: json_object` | 1/4 (truncated nested JSON — `feature` as an object, fails the `EditSpec` schema) | 3/4 (`feature: null, controls: []`) | 0 |
| gemma base, `response_format: json_object` | 0 | 4/4 | 0 |

Grammar constraints move the failure from **syntax** to **semantics** — the
reply parses but proposes nothing, which the verifier still rejects as
empty-edit. At this model scale, `response_format` buys legibility, not
acceptance.

### Against decision 0001's staging metrics

- **Local gate-pass 0% (<70% kill line):** for 135M–270M-class models,
  local-first routing is dead on arrival — *as expected*. Per decision 0002
  these numbers exercise machinery, never judge tiering; the kill thresholds
  are measured on prod-tier (Qwen3-Coder-30B-class) models. No change.
- **Sustained escalation 100%:** ditto — and the escalation path is now
  live-proven rather than mock-proven. Every rejected attempt cost one
  attempt and nothing else; no app record was ever touched by a bad edit.
- **Reconstructable from audit alone:** verified live. Example lines:
  `agent.routed — per platform default routing (pack post-op-monitor declares
  none): iterate→local` and `agent.attempt — op op-8ba9 iterate v3 tier=local
  verdict=empty-edit → climbing` followed by `tier=rules verdict=accepted →
  applied`. The full climb, policy citation included, reads back from
  `/api/apps/:id/audit` with no other source needed.
- **local p95 vs frontier:** not measurable (no frontier tier configured);
  local attempt wall was 0–8 s on 3 CPU threads.
- Pressure test **49/49 on real staging with the model tier live** (Nomad
  job registration, Vault transit round-trip, rollback, eject all green).

### Fixes this experiment needed (all committed on this branch)

1. `scripts/staging-up.sh --models` — the decision 0002 stub is now the real
   step: pinned fetch → sha256 verify → llama server on `:8081` →
   `LOCAL_MODEL_URL` guidance; idempotent; `down` stops it via the existing
   pid-file sweep.
2. `src/agent.rs` — `max_tokens: 256` on both driver request bodies. Without
   a cap, a rambling small model decodes until context exhaustion; the
   constrained protocol never needs a long reply.
3. `src/agent.rs` — `MODEL_HTTP_TIMEOUT_SECS` env override for the client
   read timeout (default unchanged at 5 s). CPU decode on small models
   legitimately exceeds 5 s; without this, a *slow* tier is misread as an
   *unreachable* one and the ladder measures timeouts, not model behavior.
4. Latent test brittleness, noted not fixed: `pressure-test.sh` asserts
   `tier=rules verdict=accepted` on an iterate. Today that passes because the
   local tier rejects everything; the day a local model produces one accepted
   edit, the assertion goes flaky. The check should accept any
   `verdict=accepted` tier when `LOCAL_MODEL_URL` is set.

## PART B — hermes-agent as an unprivileged external client

### What ran

`pip install hermes-agent==0.18.2` (PyPI, MIT) into an isolated venv worked
on Python 3.11 + Node 22 with no installer friction. Confinement for the run:
isolated `HOME` (its `~/.hermes` never touched the real home), provider
pinned to the OpenAI-compatible local endpoint (`lmstudio` provider,
`LM_BASE_URL=http://127.0.0.1:8081/v1`, dummy key), `toolsets: ["terminal"]`,
`terminal.cwd` locked to a scratch dir, `approvals.deny` globs for
`git*`/`sudo*`/`ssh*`/`rm -rf*`, `memory.memory_enabled: false`, no gateway
or messaging channels started, no external API keys present.

### How far it got: zero steps of five, twice

| Attempt | Model / handler | Outcome |
|---|---|---|
| 1 | gemma-3-270m base, model's own chat template, 5-step task with explicit URLs | One completion of hallucinated prose (fragments of hermes' own scaffold parroted back). **No tool call emitted.** |
| 2 (permitted retry) | gemma-3-270m base, llama.cpp `chatml-function-calling` grammar handler, verbatim `curl` commands to copy | One completion, prose again. **No tool call emitted.** |

The control plane's audit stream shows **no request from hermes at all** —
`/api/apps/hermes-post-op-tracker` was never created. So principle 5 ("the UI
has no privileges you don't") was *not* end-to-end exercised: the external
agent held only public-API access, but it never managed to use it. The
honest claim is therefore about the confinement (it held) and the
architecture (nothing about the API resisted third-party tooling — the task
needed only curl), not about a completed workflow.

Two structural blockers, both measured:

- **Fixed prompt footprint.** With the leanest toolset config hermes still
  ships a ~12 KB system prompt + 27 tool schemas (~46 KB JSON) ≈ 15 k tokens
  *before the task*. That excludes SmolLM2-Instruct (8 k ctx) outright — only
  the 32 k-ctx gemma could even hold the prompt, and it's a base model.
  There is no per-tool disable (only `agent.disabled_toolsets`), so the
  schema block doesn't shrink below this on 0.18.2.
- **Tool-call emission.** Sub-1B models produced no `tool_call` under either
  the native template (tools silently dropped by the server's template path)
  or the grammar-forcing `chatml-function-calling` handler.

One near-miss worth recording for decision 0002 discipline: with the model
set as a plain string (`model: "lmstudio:<file>"`) hermes silently fell back
to an AWS Bedrock default and attempted a `ConverseStream` call (failed —
no credentials; nothing billed; fixed by using the `model: {name, provider}`
mapping). **A third-party agent's "local-only" posture is one config typo
away from an external call.** For anything Wave-4-shaped, "nothing bills
anyone / nothing leaves the device" must be enforced at the egress/topology
level (the `ai-allowlist`-as-topology stance of investigation 0002), never
by trusting agent configuration.

## PART B′ — Wave-4 skills-runtime assessment (from the 0.18.2 source)

**Skills packaging.** hermes skills are agentskills.io-compatible: a
directory per skill with `SKILL.md` (YAML frontmatter: `name` ≤64 chars,
`description` ≤1024, optional `version`, `license`, `compatibility`,
`metadata`, plus `platforms` and prerequisite env-var/command declarations)
and optional `references/`, `templates/`, `assets/` subdirectories
(`tools/skills_tool.py`: "Supplementary files (agentskills.io standard)").
Skills live under `~/.hermes/skills/`; `skills.external_dirs` mounts shared
read-only skill dirs; YAML "bundles" alias N skills to one command; a
keyword/pattern security scanner runs on installed skills and
`skills.write_approval: true` stages agent-authored skill writes for human
review.

**A "deid-local pack as hermes skills" would concretely be:** a skill dir
(`deid-local/SKILL.md` carrying the de-id procedure + output contract,
`references/` for the PHI category list, `assets/` for eval fixtures), a
pinned GGUF declared via the pack's `model {}` block (investigation 0002
D3) served by llama.cpp, and a config overlay: `toolsets: ["terminal",
"file"]`, `approvals.mode: manual` + `deny` globs, `terminal.cwd` pinned,
`memory.memory_enabled: false`, provider forced to the loopback endpoint.
Every piece exists in config today — nothing needs forking.

**Compliance surface — the problem.** Memory persists to
`~/.hermes/memories/MEMORY.md` + `USER.md` and is injected into the system
prompt; it **can** be fully disabled — the config source says plainly "To
disable memory entirely, use memory_enabled: false" — and our run verified
no memory files were written. Skill self-improvement is likewise gated
(`skills.write_approval`, curator settings). Command approval has
`manual`/`smart`/`off` modes with user-editable pre-`--yolo` deny globs and
a code-shipped hardline blocklist. All good knobs. But: session transcripts
and `state.db` persist under `~/.hermes` regardless of the memory switch
(PHI in a de-id workflow would land on disk there); the default install
auto-discovers **51 plugins (44 enabled)** including web-search, image-gen
and cloud-provider integrations; the dependency tree pulls botocore, aiohttp,
PIL and dozens more; and the CLI will auto-bootstrap Node if missing. That
is an enormous audit surface for a pack whose whole pitch is "smallest
possible footprint, PHI never leaves the device" — versus the current D5
floor of one GGUF + one llama.cpp binary + our zero-dependency gates.

**License.** Runtime is MIT (`License-Expression: MIT` in the wheel
metadata). Model licenses ride separately, which our D3 `model {}` block
already anticipates (SmolLM2 Apache-2.0; the gemma file is under the Gemma
Terms — a pack shipping it must carry those terms; prefer Apache/LFM-class
weights for shipped packs).

## Changes / does-not-change

**Does not change**
- Decision 0001 — the ladder design is confirmed by first live data: wrong
  models cost attempts, never apps; audit-only reconstruction verified; kill
  thresholds remain to be measured on prod-tier models.
- Decision 0002 — the tier table stands; `--models` (consequence #1) is now
  delivered. A weak local model proved to be exactly the escalation fixture
  the decision wanted.
- Investigation 0002 D4/D5 — deid-local stays pulled forward, and the eject
  floor stays "GGUF + llama.cpp instructions", **not** a third-party agent
  runtime.

**Changes / amends**
- Decision 0002, staging weights: until the environment can reach
  huggingface.co, the pinned staging models are PyPI-published GGUFs
  (SmolLM2-135M-Instruct primary) instead of LFM2-class; same
  `LOCAL_MODEL_URL` invariance, checksums pinned in `staging-up.sh`. Swap to
  LFM2 + vendor checksums when egress allows.
- Decision 0002, enforcement stance (new corollary): "no test path may ever
  bill a frontier API" must hold against *third-party agent misconfiguration*
  too — the Bedrock near-miss shows config-level trust is insufficient;
  staging/sandbox egress should be closed at the network layer.
- Driver protocol (#4): `max_tokens` cap and `MODEL_HTTP_TIMEOUT_SECS` are
  now part of the local-tier contract (this branch).
- Wave-4 packaging idea worth adopting *without* the hermes runtime: the
  agentskills.io `SKILL.md` format as the pack's skill-packaging convention —
  runtime-agnostic, already compatible with multiple agent ecosystems.

## Recommendation

Merge the `--models` step and driver fixes; keep SmolLM2 as the default
staging rung and re-run `ladder-exercise.sh` unchanged when (a) an LFM2-1.2B
GGUF becomes fetchable and (b) a Qwen3-Coder-class endpoint exists for the
prod-tier routing spike — those two runs produce the numbers decision 0001's
kill thresholds actually bind on. Add `response_format: json_object` to the
local-tier request as the next cheap experiment (measured here: it converts
100% syntax failures into 100% verifier-legible semantic rejections). Fix
the `tier=rules verdict=accepted` pressure-test brittleness before any
capable local model lands. Do **not** adopt hermes-agent as the Wave-4 pack
runtime — its footprint and persistence surface fight the compliance story
that makes local packs valuable — but do adopt its skills *format*, and
revisit it as an external-client test harness once a ≥7B tool-calling local
model is servable in staging.
