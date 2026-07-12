# Decision 0002 — Inference test tiers: testing AI can't get expensive

Status: decided (operator directive) · Related:
[investigation 0002](../investigations/0002-local-model-simplifiers.md),
[decision 0001](0001-agent-routing.md), issues #2 and #4.

## The rule

**No test path may ever bill a frontier API.** Inference has three tiers,
selected by environment, never by test code:

| Tier | Where | What serves it | Cost |
|---|---|---|---|
| **mock** | CI, unit/contract tests, the sandbox pressure test | In-process mock OpenAI-compatible servers on ephemeral loopback ports (the pattern all round-1 treatments converged on) — scripted replies, hit counters, fault injection (invalid JSON, gate-regressing edits, dead ports) | zero |
| **liquid** | Staging (#2) | The Liquid LFM2.5 family **stitched together per task class** — 230M-class for classification/extraction probes, 1.2B/8B-A1B-instruct for constrained JSON edit proposals — served CPU-only via llama.cpp as a Nomad job on the staging pool, behind the same `LOCAL_MODEL_URL` the router already speaks | electricity |
| **prod** | Production only | Per decision 0001 / investigation 0002 D1-D2: Qwen-class local coder in-VPC + frontier (Claude, under BAA) for scaffold authorship and escalation | budgeted |

Frontier calls in staging are permitted only behind an explicit
`STAGING_FRONTIER=allow` flag with a per-run token budget, for the routing
spike measurements — never in scheduled CI.

## Why Liquid for the staging tier

- **Real inference semantics at near-zero cost.** Mocks can't produce the
  failure modes that matter to the ladder — malformed-but-plausible JSON,
  slow tokens, borderline edits that pass validity but regress a gate.
  A real small model produces them constantly, which is exactly what the
  escalation path needs to be tested against. Liquid models are open-weight,
  CPU-tolerable (230M ≈ 213 tok/s on a phone CPU), and ship llama.cpp/GGUF
  support out of the box.
- **Stitching beats one bigger model here.** Staging doesn't need one model
  that's good; it needs cheap, *differentiated* behavior per task class so
  per-tier routing and per-class escalation both get exercised. Small
  specialized instruct models per class do that; one large model would hide
  the routing seams (and cost more).
- **Vendor honesty is a feature, not a blocker.** Liquid says don't use
  these for code generation — correct, and irrelevant to the tier's job:
  in staging we are testing the *platform's* routing, verification,
  escalation, and audit behavior, not the model's code quality. A weak
  coder that triggers escalation frequently is a better staging test
  fixture than a strong one that never does.
- Double duty: the same served endpoints prototype the Wave-4 local-profile
  packs (deid/extraction), which is where LFM-class models are the
  *production* choice (investigation 0002 D4).

## Consequences

- `scripts/staging-up.sh` (#2) grows an optional `--models` step: fetch
  pinned GGUF weights, start llama.cpp server(s), export `LOCAL_MODEL_URL`.
- The router/ladder config gains nothing: tiers are already env-selected;
  environments differ only in what the URL points at. That invariance is
  the test that decision 0001's design is right.
- CI must fail any test that attempts a non-loopback model call (assert in
  the mock harness; the client already refuses TLS/off-VPC by design).
- The routing-spike kill thresholds (decision 0001) are measured on the
  **prod-tier** models in staging behind the budget flag — Liquid-tier
  numbers are for exercising machinery, never for judging model quality.
