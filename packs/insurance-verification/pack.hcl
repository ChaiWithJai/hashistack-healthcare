// Use case pack: insurance verification (RFC use case 14, wave 2 — storyboards 1b/1c)
// Staff-facing tool: nine gates including access roles, escalation path,
// and platform review, because staff queues widen the access surface.

pack "insurance-verification" {
  name        = "insurance verification"
  description = "Checks each new patient's insurance eligibility (and referral requirements) before their first visit; front desk sees a pending queue."
  profile     = "web"
  tier        = 3
  wave        = 2
  signed_by   = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  treatment_recipes = ["guided-worklist", "event-timeline", "focused-task"]

  scaffold = [
    "eligibility check form",
    "payer lookup (allowlisted endpoints only)",
    "audit log wired to every route",
    "overnight verification batch",
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

  # Historical routing values retained for signed-pack compatibility.
  # Production maps local/frontier edits to deterministic Rust rules. Gemma
  # planning uses the separate bounded workspace provider and remains the
  # only application model.
  routing = {
    scaffold    = "frontier"
    iterate     = "local"
    review      = "frontier"
    escalate_on = ["gate-regression", "invalid-edit"]
  }

  synthetic_dataset = "test patients (20 pts, synthetic payers)"
}
