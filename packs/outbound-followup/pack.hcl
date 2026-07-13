// Use case pack: outbound follow-up (RFC use case 13)
// Staff-facing queue with consent-aware outreach and clinician escalation.

pack "outbound-followup" {
  name             = "outbound follow-up"
  description      = "Consent-aware patient follow-up queue: send approved reminders, record responses, and route concerning replies to a clinician."
  profile          = "web"
  tier             = 3
  wave             = 2
  signed_by        = "platform-root-v1"
  scaffold_path    = "scaffold"
  quality_contract = "artifact-quality.json"
  treatment_recipes = ["guided-worklist", "event-timeline", "focused-task"]

  scaffold = [
    "staff follow-up queue",
    "consent check before outreach",
    "patient response capture with opt-out",
    "concerning-response escalation to clinician inbox",
    "audit log wired to every route",
  ]

  prewired = [
    "phi-encryption",
    "audit-log",
    "ai-allowlist",
    "dependency-scan",
    "synthetic-only",
  ]

  gates = [
    "phi-encryption",
    "audit-log",
    "ai-allowlist",
    "dependency-scan",
    "synthetic-only",
    "auto-logoff",
    "access-roles",
    "escalation-path",
    "human-review",
  ]

  synthetic_dataset = "outbound follow-up demo (4 synthetic patients)"
}
