# Treatment 4a — router-in-driver (composite driver, routing policy in code)

Round 1 of [the treatments ritual](../process/gitops-treatments.md), for
[#4](https://github.com/ChaiWithJai/hashistack-healthcare/issues/4) per
[investigation 0002](../investigations/0002-local-model-simplifiers.md) D1.
Branch: `claude/issue-4-treatment-a`, based on
`claude/lovable-hashistack-digitalocean-vggiqh`. Evidence, not a deliverable.

## Design in five sentences

`RouterDriver` implements the existing `AgentDriver` trait unchanged and
internally composes three drivers — `RuleBasedDriver` (existing),
`LocalDriver` (OpenAI-compatible chat-completions over plain HTTP to
`LOCAL_MODEL_URL`), and `FrontierDriver` (Claude stub, same client shape
against `FRONTIER_MODEL_URL`, never a real API) — so callers, packs, and the
doctor's workflow see one driver. Routing is a policy hardcoded in
`src/agent.rs`: `scaffold()` always routes frontier (first generation is the
first impression), `iterate()` routes local first, and no env vars at all
means pure rule-based passthrough, byte-identical to today. The router
decides: a local edit is proposed onto a *cloned* record via a constrained
JSON edit protocol, and it commits only if the reply parses and
`gates::preflight` on the clone does not regress versus before the edit;
transport failures, invalid replies, and gate regressions all discard the
clone and escalate the pristine record to the frontier driver, automatically
and invisibly. The frontier driver itself degrades to the deterministic
rule-based edit when offline or failing, so the doctor's instruction always
lands. The router records who decided what: every decision and escalation is
drained by the API layer into the audit stream as an `agent.routed` event
("iterate v2 → local (qwen3-coder) ok" / "iterate v2 → local (qwen3-coder)
failed gate-regression (5/6 → 4/6) → escalated frontier (claude-frontier)
ok").

## Footprint

`git diff --stat` vs base (`claude/lovable-hashistack-digitalocean-vggiqh`),
excluding this report:

```
 env.example              |   7 +
 src/agent.rs             | 402 +++++++++++++++++++++++++++++++++++++++++++++-
 src/api.rs               |  19 ++-
 src/state.rs             |   7 +
 tests/router_contract.rs | 355 ++++++++++++++++++++++++++++++++++++++++
 5 files changed, 786 insertions(+), 4 deletions(-)
```

- **New dependencies: zero.** The chat-completions client is std-only
  (`TcpStream`, hand-rolled HTTP/1.1 with `Connection: close`), justified
  because the endpoint is in-VPC plain HTTP by design; tests reuse the
  existing axum/tokio/tower dev stack for the mock model server.
- **New runtime moving parts: none in-process** (the router is a struct, not
  a service). The path implies one future Nomad job (vLLM/llama.cpp) per
  investigation 0002 D2, but this treatment runs with nothing extra.
- **New config surface: two env vars** — `LOCAL_MODEL_URL`,
  `FRONTIER_MODEL_URL` (documented in `env.example`). Both unset is the
  default and means today's exact behavior.
- **API/trait surface: unchanged.** `AgentDriver` is untouched; `Platform`
  gains one field (`agent_driver`); the API layer swaps its hardcoded
  `RuleBasedDriver` calls for the platform's driver plus a
  `record_routing` drain.

## Invariants kept / risked

Kept:

- **Doctor's workflow unchanged** (the disqualifier): same routes, same
  request/response shapes, no model choice ever surfaces; escalation is
  silent and the reply shape is identical on every path.
- **Every decision in the audit stream**: one `agent.routed` event per
  routing decision, actor `agent-router`, with the escalation reason inline;
  proven by tests, evaluable in CI.
- **Sandbox boundary untouched**: the router edits the app record only
  through the same structural edit shape the rule-based driver uses; gates,
  promote, restore, and export are unmodified.
- **No regression by construction**: a local edit that would worsen the gate
  report is discarded before it touches the record — the gate engine is the
  validator, not just the promotion checklist.
- **Default = today**: all 12 pre-existing tests and the 29-check pressure
  test pass unchanged with no env configured.

Risked:

- **Decision-to-audit gap**: decisions buffer in the driver until the API
  layer drains them — a new caller that forgets `record_routing` would route
  silently. (The Vault-style "no audit write, no operation" broker from #8
  would close this; a driver that writes audit directly couples agent to
  audit, which is why 4a keeps the drain.)
- **Blocking HTTP under the platform write lock**: acceptable at Phase 0
  (the lock already serializes writes) but it holds the whole platform for
  up to the client timeout (~7s worst case) per local call; staging must
  watch this.
- **Frontier results are trusted, not re-validated**: the escalation ladder
  ends at "frontier + rule-based fallback"; a frontier edit that regresses
  gates would still commit (caught later at preflight/promote, so the gate
  is not bypassable — but the addendum lands).
- **Policy is code**: changing "what routes where" is a recompile, not a
  pack or config change — the exact axis treatment 4b explores.

## What staging must measure

1. **Local gate-pass rate on pack-constrained iterate edits** — corpus of
   Wave 1 pack edits against a real Qwen3-Coder-class endpoint. Metric:
   `agent.routed … local … ok` events / all local-first attempts.
   **Threshold: ≥ 70%** (the investigation 0002 kill criterion; below it,
   local routing is theater and this path dies).
2. **Routing observability** — every `POST /api/apps` and
   `/api/apps/:id/iterate` produces exactly one `agent.routed` audit event.
   **Threshold: 100%**, checkable by diffing `/api/audit/export` counts in
   CI against request logs.
3. **Escalation completeness** — iterate calls whose addendum landed despite
   local failure (escalated or fallback) / iterate calls with a local
   failure. **Threshold: 100%** — the doctor must never see a failed edit.
4. **Iterate latency under the lock** — p95 wall time of the iterate route
   with the local endpoint live. **Threshold: p95 ≤ 2s and no other API
   route's p95 degrading > 2× while an iterate is in flight**; breach means
   the blocking-client shortcut must become async before harvest.

## What I'd steal from the other treatments

*Stub — filled during COMPOUND after reading 4b (pack-declared routing
policy) and 4c (verified escalation ladder).*
