# Platform evals — the nested-layer harness

One command proves the two claims in [docs/GOAL.md](../docs/GOAL.md) across a
realistic sampling of scenarios, and emits a portable scorecard baseline:

```bash
./scripts/evals.sh
# → docs/evals/scorecard.md (human) + docs/evals/scorecard.json (machine)
# → .evals/screenshots/ (full evidence, gitignored and published as a CI artifact)
```

- **Layer 1 — the job to be done.** Can a doctor/CHP vibe-code this tool?
  Each scenario boots its own in-memory control plane (ports 39200+) and is
  driven over real HTTP through describe → iterate → gate → fix → review →
  promote → eject. Scored: workflow completion, gate-report shape, the
  false-pass guard (promotion refused while any check fails), attestation
  presence, audit reconstructability, ejection-bundle completeness. The
  agent tier used per operation is recorded (the rules floor today — that
  honesty is part of the baseline).
- **Layer 2 — the artifact.** Is what got produced actually good? Every
  built-in pack owns `artifact-quality.json`. The ejected bundle is unpacked,
  **compiled, and run**, then a generic Playwright interpreter drives the
  pack's declared job journey. Five hard gates cover job behavior, ownership,
  safety/honesty, accessibility, and documentation. The contract—not the
  harness—contains pack-specific thresholds, selectors, and expected outcomes.

**The journey profiler is the harness's single-journey sibling**: where the
scorecard samples 78 scenarios for a regression baseline, `scripts/journey.sh`
(→ `evals/journey/profile.mjs`, process-specific high ports) runs THE flagship
journey once — describe → iterate → review the disclosed gate → enforce the
real-data lock → co-sign a synthetic demo → eject → compile, boot, and drive
the ejected app — timing every step, linking each to its audit sequence, and
emitting a narrative anyone can be
shown: [docs/evals/journey/journey.md](../docs/evals/journey/journey.md)
(+ journey.json and six stage screenshots).

**The treatment-preview proof closes the loop before export.** Run
`node evals/treatment-preview/proof.mjs` after `cargo build`. It creates three
anonymous synthetic apps, accepts each signed recipe, and drives its actual
Studio interaction in Chromium. It also checks presentation order, stable
checkpoint and audit state, zero preview-action requests, and rejection
isolation. It changes the mutable app feature list and proves the accepted
workflow remains bound to the checkpointed snapshot. The JSON report and screenshot go to the gitignored
`.evals/treatment-preview/` directory and CI uploads them as review artifacts.

Known gaps land visibly in the scorecard (today: production controls outside
the synthetic artifacts, the rules-tier floor, and the keyword shape of the
refusal screen).
The harness exits nonzero only on harness errors or a failing check in a
scenario marked `must_pass` — it is a regression baseline, not a trophy.
The four refusal scenarios (RFC 0001 use cases 9/10/15/21) are `must_pass`
since the refusal surface landed (src/refusals.rs, #12).

## The corpus (evals/scenarios/*.json)

78 scenarios across the runnable packs, with describe-phrasing variants across
personas (precise physician, colloquial physician, community health worker
in home-visit idiom, terse/typo'd — post-op-monitor, the flagship, carries
two extra), plus 4 refusal scenarios, 2 edges (duplicate app names;
restore-then-promote), and 2 identity scenarios (#10: two-tenant isolation;
staff role denial).

## Authentication (#10)

The harness is an ordinary API client, so it authenticates like one: every
request carries a bearer token from the Phase 0 dev registry
(`staging/identities.hcl`, embedded at compile time) — `dr-osei`
(clinician, meridian; the default persona), `dr-park` (clinician,
lakeside), `ms-rivera` (staff, meridian). Nothing rides the headerless dev
fallback anymore; the two auth scenarios additionally assert the tenancy
wall (cross-tenant fetch → 404, denial audited on the owning tenant's
stream), the staff promotion denial (403 + `auth.role_denied`), and that a
present-but-wrong token is 401 even in dev mode.

## Scenario schema

One JSON file per scenario. Fields by category:

### Common

| field | type | meaning |
|---|---|---|
| `id` | string | unique; also the filename stem and the scorecard row key |
| `category` | `"pack"` \| `"refusal"` \| `"edge"` | picks the harness flow |
| `persona` | string | who is phrasing the prompt (free-form label) |
| `pack` | string | pack id the doctor picks (for refusals: the nearest pack they would plausibly grab) |
| `prompt` | string | the natural-language description, verbatim — layer 1 asserts the ejected README carries it |
| `must_pass` | bool | a failing non-`expected_fail` check in a `must_pass` scenario makes the run exit nonzero |
| `notes` | string | why this scenario exists |

### `category: "pack"` (and edges)

| field | type | meaning |
|---|---|---|
| `app_name` | string | the name the doctor types (slugged into the app id) |
| `iterate` | array | conversational edits, each `{instruction, vocabulary: "rules"\|"off", expect_wired: [gate ids]}` — `rules` instructions hit the rule driver's vocabulary and must wire exactly `expect_wired`; `off` instructions must wire nothing (the honest floor: they still land as features) |
| `workflow.gate_total` | int | gates the pack demands |
| `workflow.initially_failing` | [string] | gate ids failing right after describe (order-insensitive, exact set) |
| `workflow.fixable_failing` | [string] | the subset flagged one-click fixable |
| `workflow.stubbed` | int | expected `stubbed` count (post-op's labeled encryption stub = 1) |
| `workflow.assert_false_pass_guard` | bool | try promoting while failing and demand the 409 that names the check |
| `workflow.fix_gates` | [string] | gates to fix via `POST /gate/:id/fix` after the iterations |
| `workflow.review` | bool | call `POST /review` (packs whose gate set includes `human-review`) |
| `workflow.cosigner` | string | co-signature for promote; asserted on the attestation |
| `workflow.eject_core_files` | [string] | files every bundle must contain (the nine-file core) |
| `workflow.eject_scaffold_source` | bool | converted packs must also ship `app/` source + the synthetic seed |
| `artifact` | legacy object | ignored for pack workflows; a runnable pack must carry `artifact-quality.json` in its ejected bundle |
| `edge` | string? | `"duplicate-names"` or `"restore-then-promote"` for the special flows |
| `restore_to_version` | int? | for restore-then-promote: checkpoint to restore between iterating and fixing |

### `category: "refusal"`

| field | type | meaning |
|---|---|---|
| `expected` | `"refusal-with-reason"` | the platform should refuse and say why (GOAL.md bar 7) |
| `rfc_use_case` | int | which RFC 0001 out-of-scope case this is (9, 10, 15, 21) |

The refusal surface landed (src/refusals.rs, #12): all four scenarios are
`must_pass` and assert the full contract — 422, a written reason quoting
the RFC rationale and naming the use-case class, an `app.refused` audit
event, and an empty tenant app list afterward. The screen's unit tests
(`cargo test refusals::`) additionally run **every** committed scenario
prompt through it both ways, so a new eval scenario is screened for false
positives the moment it lands.

### `category: "auth"`

| field | type | meaning |
|---|---|---|
| `auth_flow` | `"two-tenant"` \| `"staff-denial"` | picks the identity flow (#10) |
| `workflow.fix_gates`, `workflow.cosigner` | as for packs | staff-denial drives the gate green before asserting the 403/200 split |

## Adding a scenario

1. Copy the nearest existing file in `evals/scenarios/` and change `id`
   (= filename), `prompt`, `persona`, and the iterate instructions.
2. Set the workflow expectations from the pack's `pack.hcl`: `gate_total` =
   its `gates` list length; `initially_failing` = gates not in `prewired`
   (minus what your iterate instructions wire); `review: true` iff the gate
   set contains `human-review`; `stubbed: 1` only for packs whose scaffold
   carries a labeled stub (post-op-monitor).
3. Rules-driver vocabulary for `iterate` (see `src/agent.rs`): `role` →
   wires `access-roles`; `logoff` / `log off` / `idle` → `auto-logoff`;
   `escalat` / `flag` → `escalation-path`. Anything else is an off-vocabulary
   edit: a feature lands, nothing is wired — set `expect_wired: []`.
4. Run `./scripts/evals.sh`. The new row appears in both scorecards; commit
   them together with the scenario.

## Layout

```
evals/
  README.md          # this file
  scenarios/*.json   # the corpus — one self-describing scenario per file
  harness/run.mjs    # the orchestrator (Node + Playwright); invoked by scripts/evals.sh
  treatment-preview/proof.mjs # accepted-recipe browser proof before export
docs/evals/
  scorecard.md       # the portable baseline (committed, regenerated per run)
  scorecard.json     # machine twin for CI diffing
  screenshots/       # 3 committed evidence PNGs of the running ejected app
.evals/              # gitignored: bundles, logs, all screenshots, eject target dir
```

CI: `.github/workflows/evals.yml` runs the harness nightly and on demand,
installing Chromium via `npx playwright install chromium` where the dev
container's preinstalled `/opt/pw-browsers` is absent (the harness and
`scripts/evals.sh` detect which world they are in).
