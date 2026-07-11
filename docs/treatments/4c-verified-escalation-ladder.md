# Treatment 4c — verified escalation ladder with operations

Round 1 treatment for [#4](https://github.com/ChaiWithJai/hashistack-healthcare/issues/4)
(agent-driver routing, investigation 0002 D1). Branch:
`claude/issue-4-treatment-c`. Evidence, not a deliverable.

## Design in five sentences

Every agent action (scaffold, iterate, fix) becomes a Waypoint-style
**Operation** row upserted `running` on the platform state *before* any
driver runs, so a crash mid-action leaves a queryable non-terminal row
instead of silence. The supervisor climbs a fixed ladder per action — rules
→ local (`LOCAL_MODEL_URL`, OpenAI-compatible) → frontier
(`FRONTIER_MODEL_URL`, same client shape, stub) — and **no one predicts the
route**: after each attempt a deterministic verifier (gates preflight
before/after on a cloned app record, unknown-control check, non-empty-edit
check) decides accept-or-climb, so a wrong, empty, or unreachable model
costs one rejected attempt and can never corrupt an app record. The tier
drivers decide nothing about routing and the router predicts nothing about
capability — the **verifier** is the only decision-maker, and it is
model-free, pure over the app record, and identical for every rung. Every
attempt and verdict is recorded twice: structurally in the operation's
attempt list (served at `GET /api/apps/:id/operations`) and narratively in
the append-only audit stream (`agent.attempt — op op-52f0 iterate v3
tier=local verdict=gate-regression(auto-logoff lost) → climbing`).
Top-of-ladder failure marks the operation `failed` and commits nothing;
with no env vars configured the ladder is rules-only and the platform's
observable behavior is unchanged (all pre-existing tests pass unmodified).

## Footprint (honest: this is the most moving parts of the three)

`git diff --stat` vs base `origin/claude/lovable-hashistack-digitalocean-vggiqh`
(code only; this self-report adds one more doc file):

```
 src/agent.rs             | 194 +++++++++++++++++-
 src/api.rs               | 108 ++++++++--
 src/ladder.rs            | 313 +++++++++++++++++++++++++++++
 src/lib.rs               |   1 +
 src/state.rs             |  85 ++++++++
 tests/ladder_contract.rs | 499 +++++++++++++++++++++++++++++++++++++++++++++++
 6 files changed, 1188 insertions(+), 12 deletions(-)
```

- **New dependencies: zero.** The OpenAI-compatible client is ~60 lines of
  std `TcpStream` (the endpoint is loopback/in-VPC by definition); mock
  servers in tests are std `TcpListener` threads on ephemeral ports.
- **New runtime moving parts: three.** (1) the operations collection on
  `Platform` (one `Vec`, upsert by op_id), (2) the `EscalationLadder`
  supervisor + verifier, (3) `HttpModelDriver` (one struct covering both
  model tiers). Also one new API route (`/api/apps/:id/operations`) and one
  new audit action (`agent.attempt`).
- **New config surface: two optional env vars** (`LOCAL_MODEL_URL`,
  `FRONTIER_MODEL_URL`). Both unset → rules-only ladder, identical behavior;
  no other knobs, no per-pack policy file, no routing table.
- **Known warts, honestly:** the model call is a *blocking* socket inside an
  async handler while the platform write-lock is held — fine for Phase 0's
  in-memory single-lock design, a real cost the moment a slow model endpoint
  appears (5s socket timeouts bound it). Verification cost is one extra
  `gates::preflight` per attempt (cheap today; real artifact-derived gates
  in #3 make the verifier the expensive step, though also the honest one).
  Attempts re-clone the pristine record each rung, so tier outputs never
  compound — simpler, but a frontier fix of a local near-miss starts from
  scratch.

## Invariants kept / risked

- **Doctor's workflow unchanged — kept.** Same routes, same request/response
  shapes, same replies with the default ladder; escalation is automatic and
  invisible (Tao 1); the doctor never picks a model. New surface is
  additive-only.
- **Every decision in the audit stream — strengthened.** Routing was
  previously implicit (there was one driver); now every attempt, verdict,
  rejection reason, and climb is an `agent.attempt` event plus a structural
  attempt record. The operations row adds the crash story the audit stream
  alone couldn't tell: a `running`/`escalated` row with no terminal status
  IS the record of an interrupted action (proved by test: a panicking driver
  leaves a `running` op with zero attempts).
- **Sandbox boundary untouched — kept.** The ladder edits clones and commits
  only verified records; gates/deploy/audit are unmodified except one
  positional-zip reading of `preflight` output. The verifier adds a *new*
  guard: no tier can wire an unknown control or lose a passing gate.
- **Risked:** `verify_iterate` zips before/after gate reports positionally
  (safe only because `preflight` evaluates `required` in order — a sort
  added there would silently weaken the regression check); operations are
  in-memory like everything else, so the crash-visibility claim is proved
  at the state-machine level but needs the #7 Postgres row to survive a real
  process death; the empty-edit check means a legitimately no-op instruction
  ("change nothing") climbs the full ladder and fails — acceptable now,
  wrong eventually.

## What staging must measure (metric + threshold)

1. **Local-tier verified acceptance rate** on pack-constrained iterate
   instructions (the investigation-0002 routing spike): fraction of iterate
   operations where tier=local attempt verdict=accepted. **Kill: <70%**
   (0002's stated threshold) — below that the ladder is paying local-model
   latency to reach frontier anyway.
2. **Escalation tax**: p95 wall-clock of operations that climb ≥1 rung vs
   rules-only baseline. **Kill: p95 > 2× a direct frontier call** — the
   climb must cost less than the routing it avoids.
3. **Verifier honesty**: percentage of accepted edits that subsequently fail
   the *full* gate preflight at promote time. **Threshold: 0%** — a single
   verified-then-failing edit falsifies "routing emerges from verification."
4. **Crash visibility**: kill -9 the control plane mid-iterate under load;
   on restart, count interrupted actions not represented by a non-terminal
   operation row. **Threshold: 0 missing** (requires the #7 durable store;
   in-memory staging can only prove the upsert-before-work ordering, which
   the test suite already pins).

## What I'd steal from the other treatments

*(to fill during COMPOUND, after reading 4a and 4b's self-reports)*

- Expected from 4a (router-in-driver): whatever it learned about keeping the
  driver trait the sole seam — if its router needs no supervisor loop, its
  call-site footprint is the bar to beat.
- Expected from 4b (pack-declared policy): the pack is the right place to
  *shorten* the ladder per use case (`with_tiers` was left public for
  exactly this); a pack-declared tier list feeding this treatment's verifier
  may be the harvest shape.
