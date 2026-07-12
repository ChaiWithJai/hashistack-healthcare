// Use case pack: hypertension tracker (RFC use case 1, wave 1 — launch pack)

pack "hypertension-tracker" {
  name        = "HTN tracker"
  description = "Home blood-pressure logging for hypertensive patients with trend view and out-of-range flags routed to the clinician."
  profile     = "web"
  tier        = 3
  wave        = 1
  signed_by   = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"

  scaffold = [
    "BP entry form (patient-facing)",
    "trend chart with target band",
    "out-of-range flag to clinician inbox",
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
    "escalation-path",
  ]

  synthetic_dataset = "HTN demo (15 pts, 90 days of readings)"
}
