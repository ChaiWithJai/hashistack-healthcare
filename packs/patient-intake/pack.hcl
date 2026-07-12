// Use case pack: patient intake (RFC use case 6, wave 1 — launch pack)

pack "patient-intake" {
  name        = "patient intake"
  description = "Pre-visit intake forms patients complete at home; structured summary lands in the chart before the appointment."
  profile     = "web"
  tier        = 3
  wave        = 1
  signed_by   = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"

  scaffold = [
    "intake form builder (history, meds, allergies)",
    "patient link with expiring token",
    "structured summary for the chart",
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

  synthetic_dataset = "intake demo (10 pts)"
}
