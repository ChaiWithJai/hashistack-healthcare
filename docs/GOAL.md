# The goal and the bar

## The goal

**The end user is a doctor or community health professional (CHP) who can
vibe-code one of the healthcare use cases we described — and then walk away
owning it.**

Concretely, two moments have to work:

1. **Vibe-code it.** A clinician with no engineering background describes one
   of the seeded use cases in natural language ("a post-op recovery tracker
   for my knee replacement patients"), picks a pack, and gets a running,
   HIPAA-scaffolded app in a sandbox. They iterate conversationally, the
   compliance gate is cleared as part of building (not as a wall at the end),
   they co-sign, and it goes in front of real patients under the BAA
   boundary.

2. **Own and extend it.** What they built is *theirs* — like they just made
   their own customizable template that they like and now want to extend.
   They can eject at any time and receive a self-contained, documented
   repository: their app's source, **their documentation generated from
   their own record** (prompt, addenda history, gate report, attestation,
   audit excerpt, the pack's clinical evidence citations), and deploy
   manifests for Nomad/Render/Fly/Kamal. No hostage code, no hostage docs.
   The ejected app is itself a pack-shaped template: re-importable,
   shareable with their practice, submittable to the registry.

The platform's job is the loop between those two moments. Tracked as the
ejection ticket (#11) and the use-case enablement investigation (#12).

## The bar

Each seeded use case counts as **enabled** only when all of these hold in the
staging environment (#2) — verified by the pressure test, not by manual smoke
testing:

| # | Bar | Verified by |
|---|-----|-------------|
| 1 | Natural-language description + pack → running sandbox app on synthetic data, no hand-edits | staging pressure test (#2), agent driver (#4), runnable scaffolds (#5 — post-op-monitor converted and CI-tested; four packs pending), eval harness layer 1 across 4+ personas per pack (`scripts/evals.sh` → [evals/scorecard.md](evals/scorecard.md)) |
| 2 | The app cannot reach real data while any gate fails; the failure is named and, where safe, one-click fixable | false-pass guard (tested today), evidence-based gates (#3) |
| 3 | Promotion requires a clinician co-signature and produces an attestation bound to the gate report | tested today; cryptographic binding in #10 |
| 4 | Every action lands in one append-only audit stream, exportable for a security review | tested today; durable + load-bearing in #8 |
| 5 | Eject produces a repo a stranger can run from the included docs alone | ejection ticket (#11), eval harness layer 2: the ejected bundle is unpacked, built, and RUN, then driven with Playwright ([evals/scorecard.md](evals/scorecard.md)) |
| 6 | The ejected app works as the clinician's own template: re-import, extend, share | ejection ticket (#11), pack spec (#5 — pattern set by post-op-monitor: ejected bundles carry its real scaffold source), eval harness layer 1: every scenario's bundle must carry the derived pack.hcl and the doctor's own prompt ([evals/scorecard.md](evals/scorecard.md)) |
| 7 | Out-of-scope use cases (RFC: 9, 10, 15, 21) are refused **with a written reason** in the product | enablement investigation (#12) |

The demo in this repo proves bars 2–4 as *contracts* over a simulated
platform. The ticket chain (#2–#11) makes each contract true of real
infrastructure without changing the workflow the clinician sees — that
invariance is the point of the Tao's "workflows, not technologies."

## Non-goals (for now)

- Building for engineers. The CLI and hospital integrations are API clients
  we get for free (principle 5); the design target stays the clinician.
- The four refused use cases (enterprise outcomes, ONC interoperability,
  triage, FDA device). Refusal with a reason is a trust feature.
- Multi-region, marketplace economics, and colo math — RFC Phase 3.
