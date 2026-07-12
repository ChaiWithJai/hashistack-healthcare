# Pack-specific gate — escalation-path semantics for post-op recovery

**Status: documentation only.** The gate engine's pack-gate plugin API is
future work tracked in issue #3; this file specifies the check this pack
will register the day that API exists, so the semantics are reviewed and
signed with the pack now rather than invented later.

## The check: `post-op-escalation-path`

> A check-in flag over threshold must **route to the practice inbox** —
> a queue a human is accountable for — not merely render on a dashboard.

Concretely, on the scaffolded app the gate must verify:

1. **Threshold semantics.** Pain at or above 7 (0–10 scale), or a wound
   reported as `drainage`, `opening`, or `spreading-redness`, produces an
   escalation flag. (The scaffold implements exactly this:
   `scaffold/src/main.rs`, `PAIN_ESCALATION_THRESHOLD` and
   `CONCERNING_WOUND_STATUSES`; its tests assert both directions.)
2. **Routing, not display.** The flag lands in the practice inbox queue —
   an addressable, auditable destination — synchronously with the check-in
   that raised it. A flag that only appears in a UI fails the check.
3. **No silent drop.** A check-in that raises a flag and a flag reaching
   the inbox appear as distinct audit events, so a lost flag is visible in
   the export.

## Relationship to the platform's `escalation-path` gate

The platform ships a generic `escalation-path` control gate ("no escalation
path for out-of-range or urgent findings" — `src/gates.rs`); packs like
insurance-verification already require it. This pack's check is the
*domain-specific sharpening*: it does not ask "does an escalation path
exist" but "do post-op thresholds route where this practice answers them".
When the plugin API lands, `post-op-escalation-path` is added to this
pack's `gates` list in `pack.hcl` alongside — not replacing — the generic
control.

## Why it is a gate and not a feature

A dropped escalation is the exact failure mode that turns a helpful
recovery tracker into a liability: the patient believes the practice saw
their drainage report. Making it a promotion gate means an app edit that
unwires inbox routing is caught by the ladder's verifier as a gate
regression (decision 0001) before it can reach a released app.
