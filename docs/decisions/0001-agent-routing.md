# Decision 0001 — Agent routing architecture (#4, treatment round 1)

Status: decided · Inputs: treatments 4a/4b/4c (branches `claude/issue-4-treatment-{a,b,c}`,
self-reports in [docs/treatments/](../treatments/) on those branches) ·
Rubric: [investigation 0001](../investigations/0001-enable-all-use-cases.md) ·
Framing: [investigation 0002](../investigations/0002-local-model-simplifiers.md) D1.

## Verdict

**Harvest 4c's spine, 4b's policy surface, 4a's restraint.**

The winning architecture is the **verified escalation ladder** (4c): every
agent action is an Operation upserted `running` before any driver runs, a
supervisor climbs rules → local → frontier, and a *deterministic, model-free
verifier* (gate preflight delta on a cloned record + cheap validity checks)
decides accept-or-climb after every attempt. Routing emerges from
verification, not prediction — which is the only stance consistent with a
platform whose product is the gate.

Grafted from 4b: **tier selection and escalation consent are signed pack
content** — an optional `routing` attribute in `pack.hcl` (first tier per
action, `escalate_on` list), with platform defaults when absent. This makes
routing policy reviewed, diffable, CI-evaluable, and citable per audit event,
and it operationalizes 0002's compounding insight (the pack that constrains
harder can route lower).

Grafted from 4a: restraint — one `HttpModelDriver` struct for both local and
frontier tiers, no separate service, env-only config, and the pure
passthrough default (no env vars → rules-only, byte-identical to today).

## Why, in rubric order

1. **Doctor's workflow:** unchanged in all three — tie.
2. **Compliance evaluability:** decisive. 4c makes *behavior* evidence
   (crash-visible operations, every attempt + verdict audited); 4b makes
   *policy* evidence (signed content, loud parse failures). Combined, both
   the rule and the conduct are auditable — neither alone covers HIPAA's
   "prove what happened AND what was supposed to happen." 4a keeps policy in
   code where only a code review can see it.
3. **Footprint:** 4a wins raw (+786 lines vs +1176/+1309), but the RFC
   already committed to Waypoint-style operations (Appendix A, amendment 4)
   — 4c's ops table is planned platform structure, not incidental weight.
   The harvest trims the union hard; target ≤ 4c's footprint.
4. **Margin/ops:** tie (all three: zero deps, in-VPC plain HTTP, one future
   model-serving Nomad job).
5. **Eject cleanliness:** 4b's pack-declared policy travels with the ejected
   template; 4c/4a neutral. Grafting 4b wins this row too.

The emergency-override question 4b's self-report raised resolves cleanly in
the combined design: the **ladder (platform code) is the outer authority**;
pack policy expresses *consent within it* (which tier first, which failures
may spend frontier tokens); a platform override outranks pack content by
construction and is itself an audited config change.

## What the losers taught (recorded before deletion)

- 4a: escalate-inside-the-driver keeps the API layer honest — the harvest
  keeps its drain-style audit recording and its "frontier degrades to rules
  when offline, the doctor's edit always lands" guarantee.
- 4b: enum-typed policy fields make bad config fail at registry load as
  loudly as a bad signature — port that exact behavior.
- Shared discovery (all three independently built the same ~60-line std-only
  OpenAI-compatible client and the same clone-then-preflight verifier):
  those are the round's **base primitives**, true regardless of winner.

## Staging metrics (union of the three kill thresholds)

Local gate-pass < 70% on pack-constrained edits → kill local-first routing;
sustained escalation > 30% → rethink tiering; any decision not
reconstructable from the audit stream alone → bug, fix before scale;
local p95 > 1.5× frontier → the economics don't pay.
