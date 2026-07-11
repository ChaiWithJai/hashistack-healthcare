# Platform evals — the nested-layer harness

One command proves the two claims in [docs/GOAL.md](../docs/GOAL.md) across a
realistic sampling of scenarios, and emits a portable scorecard baseline:

```bash
./scripts/evals.sh
# → docs/evals/scorecard.md (human) + docs/evals/scorecard.json (machine)
# → .evals/screenshots/ (full evidence, gitignored; 3 best committed under docs/evals/screenshots/)
```

- **Layer 1 — the job to be done.** Can a doctor/CHP vibe-code this tool?
  Each scenario boots its own in-memory control plane (ports 39200+) and is
  driven over real HTTP through describe → iterate → gate → fix → review →
  promote → eject. Scored: workflow completion, gate-report shape, the
  false-pass guard (promotion refused while any check fails), attestation
  presence, audit reconstructability, ejection-bundle completeness. The
  agent tier used per operation is recorded (the rules floor today — that
  honesty is part of the baseline).
- **Layer 2 — the artifact.** Is what got produced actually good? For packs
  with a runnable scaffold (post-op-monitor today, #5), the ejected bundle
  is unpacked, **compiled, and run** (ports 39300+, one shared worktree-local
  `CARGO_TARGET_DIR` so it compiles once), then judged with Playwright
  against the running ejected app: it renders (form fields, SYNTHETIC
  banner, the sketchy-kit skin from `web/index.html`), it does the clinical
  job (a pain-9 check-in routes a flag to the practice inbox and the stdout
  audit JSONL; a pain-2 does not), and it keeps its honesty markers (the
  encryption stub is labeled, never claimed). Unconverted packs score
  **no-artifact (#5 pending)** — visible in the scorecard, never skipped.

Known gaps are **expected failures**, not errors: the four refusal
scenarios (RFC 0001 use cases 9/10/15/21) run against a platform with no
refusal surface (#12) and fail into the scorecard's known-gaps section.
The harness exits nonzero only on harness errors or a failing check in a
scenario marked `must_pass` — it is a regression baseline, not a trophy.

## The corpus (evals/scenarios/*.json)

30 scenarios: for each of the 5 packs, 4+ describe-phrasing variants across
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
| `artifact` | object | `{expected: "playwright", checks: [...]}` to run layer 2, or `{expected: "no-artifact", reason}` to score the gap visibly |
| `edge` | string? | `"duplicate-names"` or `"restore-then-promote"` for the special flows |
| `restore_to_version` | int? | for restore-then-promote: checkpoint to restore between iterating and fixing |

### `category: "refusal"`

| field | type | meaning |
|---|---|---|
| `expected` | `"refusal-with-reason"` | the platform should refuse and say why (GOAL.md bar 7) |
| `rfc_use_case` | int | which RFC 0001 out-of-scope case this is (9, 10, 15, 21) |

The refusal check is marked `expected_fail` in the results until the
refusal surface (#12) lands — when it does, flip `must_pass` to `true` and
the baseline starts protecting it.

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
