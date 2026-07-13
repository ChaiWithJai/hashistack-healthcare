# Product Use Case

## Customer
Clinicians who are already vibe-coding practice tools (post-op trackers, intake
forms, insurance checkers) on consumer platforms — and the practices and
hospitals that have to answer for where the patient data went. Every platform
they use today fails at the same two points: no BAA, no path from prototype to
compliant deployment.

## Job
Describe a tool in natural language and get a running, HIPAA-scaffolded app —
then get it safely from "works on synthetic data" to "in front of real
patients" without hiring a compliance team. The workflow is the fixed contract:
describe → generate → preview → iterate → **gate** → deploy → operate → audit.

## Managed Default
Lovable / a consumer app builder on Fly or Vercel, or plain Supabase +
Cloudflare glue. That is the right answer for Tier 1–2 tools (no PHI). It
proves the workflow cheaply — and stalls exactly at the Tier 3 wall: the gate
step doesn't exist, so responsible deployment is discipline instead of code.

## Rust-Owned Risk
The **gate engine** and the **audit pipeline** — the two places where a wrong
answer is a reportable incident:

- A gate verdict must be reproducible evidence: same app state in, same verdict
  out, no false pass. The promotion path must be impossible to reach with a
  production blocker (`tests/platform_contract.rs::gate_blocks_real_data_then_admits_disclosed_synthetic_demo`).
- The audit stream must be append-only and complete: every scaffold, edit, gate
  result, deploy, and rollback lands in one exportable sequence
  (`audit_stream_records_the_whole_story_append_only`).

Typed state machines, exhaustive enums, and a compiler that refuses ambiguity
are the reason this slice is owned Rust rather than glue.

## Transfer
The same gate-before-promotion pattern transfers to any regulated
generate-then-deploy loop: fintech (SOC 2 / PCI gates on generated internal
tools), legal (privilege screens), education (FERPA). Swap the pack set and the
gate registry; the control plane and workflow contract stay.
