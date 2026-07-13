// Use case pack: compliance checklist (RFC use case 4, wave 1 — launch pack)
// Tier 2: practice-facing, no patient-facing surface; the lowest-risk launch pack.

pack "compliance-checklist" {
  name        = "compliance checklist"
  description = "Practice-facing HIPAA task tracker: recurring safeguards, training due dates, and evidence attachments for audits."
  profile     = "web"
  tier        = 2
  wave        = 1
  signed_by   = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  treatment_recipes = ["guided-worklist", "event-timeline", "focused-task"]

  scaffold = [
    "safeguard checklist with recurrence",
    "training due-date tracker",
    "evidence attachment store (encrypted)",
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
    "auto-logoff",
    "synthetic-only",
  ]

  synthetic_dataset = "sample practice (1 clinic, 8 staff)"
}
