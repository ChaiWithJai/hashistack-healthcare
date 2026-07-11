# The treatments ritual — gitops for contested decisions

When a ticket embeds a real architectural choice, don't debate it — **run
treatments**: parallel, disposable implementations of each candidate path,
each on its own branch, judged against the verified substrate, harvested for
primitives. Git is the lab notebook; branches are the petri dishes.

## The loop

```
1. FRAME    decision + candidates + kill criteria (a decision record stub,
            usually born in an investigation doc)
2. TREAT    one branch per candidate: claude/issue-<N>-treatment-<x>
            · built by parallel agents in isolated worktrees
            · same base commit, same test gates (fmt/clippy/test/pressure-test)
            · each ships docs/treatments/<N><x>.md — a structured self-report
3. COMPOUND read all self-reports; extract the base primitives every
            treatment independently needed (those are real regardless of
            winner) onto the integration branch
4. JUDGE    when the verification substrate is ready (staging, #2), score
            each treatment with the investigation-0001 rubric plus the
            footprint metrics below; write the decision record
5. HARVEST  reimplement the winner cleanly on the integrated base (never
            merge a treatment branch raw — treatments are evidence, not
            deliverables); delete losing branches after recording what
            they taught
6. REPEAT   next contested ticket
```

## Self-report format (docs/treatments/<N><x>.md)

- **Design in five sentences** — what routes where, who decides, who records
- **Footprint**: `git diff --stat` vs base, new dependencies, new runtime
  moving parts, new config surface
- **Invariants kept/risked** — especially: doctor's workflow unchanged,
  every decision in the audit stream, sandbox boundary untouched
- **What staging must measure** to prove or kill this path (exact metric +
  threshold)
- **What I'd steal from the other treatments** (filled during COMPOUND)

## Judge rubric (in order, from investigation 0001)

1. Does the doctor's workflow change? (disqualifying)
2. Is the compliance story evaluable in CI?
3. Footprint: fewest moving parts / deps / config for the same bar
4. Margin vs ops ownership
5. Eject cleanliness

## Live rounds

| Round | Decision | Treatments | Status |
|---|---|---|---|
| 1 | #4 agent-driver routing architecture (per investigation 0002 D1) | 4a router-in-driver · 4b pack-declared routing policy · 4c verified escalation ladder | harvested → [decision 0001](../decisions/0001-agent-routing.md) implemented in `src/ladder.rs` (verified: 29 tests, 39/39 sim + 47/47 staging-backed pressure checks); treatment branches superseded — lessons live in the decision record (ref deletion returns 403 under this environment's Git credentials; delete `claude/issue-4-treatment-{a,b,c}` from the GitHub UI at leisure) |
