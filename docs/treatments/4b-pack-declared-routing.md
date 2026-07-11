# Treatment 4b — pack-declared routing policy

Round 1 of [the treatments ritual](../process/gitops-treatments.md), issue
[#4](https://github.com/ChaiWithJai/hashistack-healthcare/issues/4), per
[investigation 0002](../investigations/0002-local-model-simplifiers.md) D1/D3.
Branch: `claude/issue-4-treatment-b`. Evidence, not a deliverable.

## Design in five sentences

The signed pack manifest grows an optional `routing` attribute
(`routing = { scaffold = "frontier", iterate = "local", review = "frontier",
escalate_on = ["gate-regression", "invalid-edit"] }`) with platform defaults
when absent, so *where each agent operation runs* is codified, reviewed, and
signature-covered exactly like the gate list — codification over code. A
deliberately dumb `Dispatcher` reads that policy and calls the named tier:
`rules` (the existing `RuleBasedDriver`), `local` (`LocalDriver`, an
OpenAI-compatible plain-HTTP client at `LOCAL_MODEL_URL`), or `frontier`
(`FrontierDriver`, the identical client shape at `FRONTIER_MODEL_URL` — a
stub standing where the real ClaudeDriver lands). The pack decides:
escalation fires only for the failure classes the pack names in
`escalate_on` (a local edit that would unwire a satisfied gate, or a
malformed reply), while an unlisted failure falls back to the deterministic
rules floor so the doctor's workflow never dies and frontier tokens are
never spent without pack consent. Every decision — routed, escalated, or
fallen back — lands in the audit stream as `agent.routed` /
`agent.escalated` citing its policy source ("per pack insurance-verification
routing policy: iterate→local" vs "per platform default routing (pack
post-op-monitor declares none)"), so the export answers "who decided" by
itself. With no env vars set every tier resolves to `RuleBasedDriver`,
which is why the pre-existing contract tests and the pressure test pass
unchanged.

### The HCL shape, and why

A plain object attribute, not a labeled block:

```hcl
routing = {
  scaffold    = "frontier"
  iterate     = "local"
  review      = "frontier"
  escalate_on = ["gate-regression", "invalid-edit"]
}
```

hcl-rs maps an object expression straight onto a serde struct with enum
fields — no custom block-body plumbing — and Packer uses the same
attribute-object shape for `required_plugins` (steering §3). Serde enums
(`RoutingTier`, `EscalationReason`) mean a typoed tier or reason fails at
registry load, the same loud path as the signature check: a control plane
with a half-understood routing policy never boots. When pack.hcl v2 grows
Waypoint-style stage blocks (`generate {}`, `gate {}` — steering §4), this
attribute folds into a `use`-per-stage shape naturally; nothing here fights
that migration.

## Footprint

`git diff --stat` vs base (`origin/claude/lovable-hashistack-digitalocean-vggiqh`):

```
 docs/treatments/4b-pack-declared-routing.md | (this file)
 packs/insurance-verification/pack.hcl       |  13 ++
 src/agent.rs                                | 395 ++++++++++++++++++++++-
 src/api.rs                                  |  60 +++-
 src/packs.rs                                | 176 +++++++++++
 tests/routing_contract.rs                   | 405 ++++++++++++++++++++++
 5 files changed, 1038 insertions(+), 11 deletions(-)
```

- **New dependencies: zero.** The OpenAI-compatible client is ~60 lines of
  std `TcpStream` HTTP/1.1 (POST, `connection: close`, no TLS/chunked/
  streaming) — enough for an in-VPC endpoint and the mock-server tests. The
  real frontier client needs a proper HTTP stack; that cost is deferred, and
  honest to defer, because the frontier tier is a stub by design here.
- **New runtime moving parts: one** — the `Dispatcher` (rules + optional
  local/frontier clients), constructed once from env at router build.
- **New config surface:** `LOCAL_MODEL_URL`, `FRONTIER_MODEL_URL` env vars;
  one optional `routing` attribute in pack.hcl; platform defaults constant
  (`RoutingPolicy::default`: scaffold=frontier, iterate=local,
  review=frontier, escalate_on=[]).

## Invariants kept / risked

- **Doctor's workflow unchanged — kept.** No new routes, no new UI, no model
  picker. Routing and escalation are invisible; on any failure the rules
  floor keeps the iterate loop alive (Tao 1, investigation 0002 bar guard 2).
- **Every decision in the audit stream — kept, strengthened.** Each
  routed/escalated/fallback decision is a first-class audit event whose
  detail names the policy that produced it. The JSONL export alone now
  reconstructs which model tier touched which app version and *why*.
- **Sandbox boundary untouched — kept.** The dispatcher changes who authors
  an edit, never where the app runs or what data it sees; a gate-regressing
  local edit is discarded before it reaches the record, and even an accepted
  removal is re-caught by preflight before promotion — the gate engine stays
  the backstop.
- **Does moving routing into signed pack content strengthen or complicate
  the compliance story? Strengthens it, with one honest complication.**
  Strengthens: "which AI touches PHI-adjacent work" stops being an opaque
  runtime heuristic and becomes reviewed, signed, versioned, diffable pack
  content — evaluable in CI (the registry load test IS the policy lint), and
  citable per decision in the audit export. That is the same move that made
  gates trustworthy, and it extends investigation 0002 D3 ("a model is pack
  content") to "a routing policy is pack content." The complication: policy
  now ships per-pack, so a platform-wide emergency ("stop routing to local
  until CVE-X is patched") requires re-signing packs or a platform override
  knob that *outranks* signed content — and that override, once it exists,
  must itself be audited or the signature story quietly weakens. Treatment
  4a/4c should be judged on exactly this point in COMPOUND.
- **Risked: policy sprawl.** Five packs is fine; fifty packs with bespoke
  escalation lists is a review burden. Mitigation already in shape: defaults
  are platform-owned and packs only declare deltas (one of five overrides).

## What staging must measure

1. **Local gate-pass rate** on pack-constrained iterate instructions routed
   per policy against a real Qwen3-Coder-30B-class endpoint: **kill below
   70%** (investigation 0002's routing-spike threshold), target ≥80%.
2. **Escalation rate** (agent.escalated / agent.routed iterate events, read
   straight from the audit export): **kill above 30%** sustained — above
   that the local tier is theater and the two-endpoint complexity isn't
   earning its keep.
3. **Audit reconstructability:** for 100% of sampled iterate addenda, the
   export alone must answer "which tier authored this version and under
   which policy" — any gap kills the treatment's whole premise.
4. **p95 iterate latency** local vs frontier path: local must not exceed
   frontier p95 by more than 1.5× or the routing saves money by spending
   the doctor's patience.

## What I'd steal from the other treatments

*(to fill during COMPOUND, after reading 4a and 4c self-reports)*

- From 4a (router-in-driver): likely its confidence/heuristic signals as
  additional `escalate_on` vocabulary — the pack names the policy, the
  driver contributes richer failure classes.
- From 4c (verified escalation ladder): likely its verification step between
  tiers — this treatment trusts the frontier retry blindly and re-checks
  only at preflight.
