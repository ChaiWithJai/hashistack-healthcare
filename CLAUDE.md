# CLAUDE.md — working agreements and operator profile

This file is ambient memory. It records how the operator (Jai) works and
decides, observed across sessions, so agents calibrate their own gating —
what to decide, what to record, what to escalate. Append signals as they
appear; never ask the operator to restate something recorded here.

## Operator review profile (observed, cumulative)

- **Honest labeling beats impressive labeling.** The HITL checkpoint on the
  first PR was "this is a skinned UI" — the response that worked was
  squashing to signal and describing the work as exactly that. Never let a
  commit, PR, or doc imply more reality than exists; put the Real/Simulated
  split in the artifact itself.
- **Verification must be automatic and virtual.** "We can't verify anything
  unless it's manual smoke testing" is a rejected state. Every claim needs a
  pressure-test assertion or a CI gate; staging must be spinnable without a
  cloud account.
- **Work is ticketed, and tickets are wired into the artifact** — referenced
  in the PR body AND as TODO(#n) at the exact simulation/shortcut points in
  source.
- **Cost discipline on AI is a hard rule, not a preference.** Testing AI
  can't get expensive: mock in CI/sandbox, small local models (Liquid-class,
  stitched) in staging, frontier only in prod behind explicit budget flags
  (decision 0002).
- **Prefers parallel exploration with gitops** (treatments ritual): try
  contested paths as disposable branches, compound the shared primitives,
  judge against verified substrate, harvest the winner, delete the rest.
  Minimal footprint is the tiebreaker.
- **Wants autonomy between checkpoints.** "Rinse and repeat so I can enjoy
  the rest of my day." Keep loops self-driving; interrupt only for merge
  decisions, credentials, or scope changes.
- **Prefers ambient recording over being polled.** Don't end review stops
  with question lists; make the call, record the rationale and the
  provisional decision in docs/process/review-log.md, and let vetoes come
  asynchronously. (Stated explicitly 2026-07-11: "This stuff should be
  recorded ambiently by you and then you'll know how I deal with [it] to
  improve gating.")
- **End goal framing:** the user is a doctor/CHP who vibe-codes a described
  healthcare use case, then ejects and owns it as their own extendable
  template (docs/GOAL.md). Judge every decision against that bar and the
  Tao (workflows over technologies; the gate is the product).

## Standing mechanics

- Commit discipline: scope-prefixed subjects (`gates:`, `packs:`, `agent:`,
  `deploy:`, `audit:`, `ui:`, `infra:`, `docs:`, `staging:`, `eject:`,
  `strategy:`), behavior described as user-visible fact.
- Branches: one branch per ticket/thread (`claude/issue-N-*`); treatments as
  `claude/issue-N-treatment-x`; never push to another thread's branch.
  Ref deletion is denied (403) under this environment's credentials —
  mark superseded branches in docs instead.
- Gates before any commit: `cargo fmt --check`, `clippy -D warnings`,
  `cargo test`, `./scripts/pressure-test.sh`.
- Shared CARGO_TARGET_DIR across parallel same-crate agents causes
  cross-contamination — parallel agents use worktree-local target dirs.
- Decisions live in docs/decisions/ (numbered), investigations in
  docs/investigations/, review dispositions in docs/process/review-log.md.
