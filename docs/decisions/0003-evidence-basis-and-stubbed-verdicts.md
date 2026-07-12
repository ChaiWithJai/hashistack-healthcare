# 0003 — Evidence basis and the `stubbed` verdict shape (#3)

Status: provisional (operator veto welcome — review-log posture)
Date: 2026-07-11
Tickets: #3; implements review-log round-1 items P1 (dual-register
vocabulary), F3 (frozen attestation-time report), F1 (rendered-job vault
stanza assertion)

## Problem

Gates evaluated a `controls` set the scaffold/agent self-reports on the app
record. A buggy or malicious generator could set `phi-encryption` without
encrypting anything: the gate report was a claim, not evidence. Separately,
the post-op scaffold's encryption is an honestly-labeled stub — so a naive
"evidence" pass faced a fork: report the stub as *pass* (a false pass, the
exact failure mode #3 exists to kill) or as *fail* (blocking every
promotion until a real hipaa-core cipher exists, i.e. removing the demo
before design partners — vetoed by P3).

## Decision

1. **Per-gate `basis` field.** Every gate result says what it rests on:
   `control` (self-reported wired-control set — a claim) or `evidence`
   (static analysis over the pack's compile-time-embedded scaffold sources).
   Packs without a runnable scaffold keep control-basis verdicts unchanged.

2. **Evidence is an optional capability, discovered per gate** — a separate
   `Evidence` trait next to `Gate`, probed via `Gate::as_evidence()` with a
   NotSupported (`None`) default, the way Nomad probes plugins for optional
   interfaces. `Gate::evaluate` stays the pure/cheap validate phase;
   `Evidence::inspect` is the execute phase of Packer's prepare/run split
   (steering §3). Both are side-effect-free: Phase 0 evidence is textual
   analysis, never execution, so the whole gate plan stays dry-runnable.

3. **The `stubbed` verdict — the honest middle.** Where evidence finds the
   control's plumbing but the mechanism is a labeled placeholder (the
   scaffold's encryption stub), the verdict is `status:"stubbed"` with the
   fields and the TODO named. Semantics, precisely:
   - never rendered or counted as `pass` — `passed` counts strict passes
     only; the report carries a separate `stubbed` counter, the summary
     string discloses it (`5/6 (1 stubbed)`), and it flows verbatim into
     the attestation, the audit line, COMPLIANCE.md, and the UI meter;
   - satisfies promotion in Phase 0 (`green` = zero failures): the whole
     substrate is labeled simulation (P3), and an honestly-labeled stub is
     exactly the Real/Simulated line the operator ships on. A report can
     therefore be green while loudly not claiming encryption happened;
   - flipping stubs to blocking later is one line (`GateStatus::satisfied`),
     and the recorded verdicts already distinguish the two.

4. **No-marker is a failure, not a pass.** The PHI walker (`// phi:` field
   markers + `// phi-encryption:` struct dispositions, convention defined in
   gates.rs) fails a scaffold with no PHI inventory, an undeclared field, an
   unknown disposition, or a `vault-transit` claim without a call site. The
   absence of evidence is never evidence.

5. **Textual limits stated, not hidden.** The route walker reads one axum
   builder chain per file (no merge/nest, no cross-function routers); the
   host scan sees literals only (dynamic URLs need the observed-egress
   evidence still queued under #3); the PHI walker trusts marker placement.
   What none of them can do is upgrade a stub or a rogue artifact to a pass
   — the adversarial fixture (tests/evidence_contract.rs) pins that.

6. **P1:** a static table in gates.rs maps gate ids to HIPAA citations
   (45 CFR §164.312(b) etc.); report JSON and COMPLIANCE.md carry them; UI
   text unchanged; pack-defined clinical gates (escalation-path) honestly
   carry none.

7. **F3:** `deploy::promote` freezes the admitting `GateReport` on the
   attestation; a released app's COMPLIANCE.md embeds that verbatim
   ("frozen at promotion") instead of re-running preflight over
   reconstructed sandbox lineage. Drafts keep the live re-run. The lineage
   fallback survives only for live records predating stored reports.

8. **F1:** a pressure-test assertion proves the rendered job text in the
   export bundle always contains the `vault {` stanza, even while dev-mode
   staging strips it at submission.

## Consequences

- The post-op gate report is now `4 passed, 1 stubbed, auto-logoff failing`
  before the fix and `5/6 (1 stubbed)` green after — every consumer
  (pressure test, contract tests, attestation, UI) discloses the stub
  instead of absorbing it. `6/6` claims are gone wherever a stub exists.
- The fonts hosts the scaffold's skin loads are now *declared* in the
  pack's signed network-allowlist (`asset_endpoints`, no-PHI browser
  fetches, trade-off named in the policy file) — the ai-allowlist evidence
  pass found them undeclared, which is the gate working as intended.
- Still open under #3: a real dependency scanner behind an Evidence impl,
  egress observed from the sandbox allocation, and evidence coverage for
  the other four packs as #5 converts them.
