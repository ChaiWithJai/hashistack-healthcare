// Use case pack: post-op monitor (RFC use case 2, wave 2 — pulled forward for storyboard 1a)
// A pack is declarative, signed, and versioned — the platform's extension unit,
// same philosophy as a Terraform module or a Nomad job spec.

pack "post-op-monitor" {
  name        = "post-op monitor"
  description = "Recovery tracking for surgical patients: daily pain + wound check-ins, encrypted photo upload, escalation flags to the practice inbox."
  profile     = "web"
  tier        = 3
  wave        = 2
  signed_by   = "platform-root-v1"

  # This pack ships a real runnable app template (issue #5): scaffold/ is a
  # standalone axum crate seeded from synthetic/, ejected as the app source.
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  static_evidence = true

  # What the agent scaffolds before the doctor's first edit, hipaa-core pre-wired.
  scaffold = [
    "pain + wound check-in form",
    "photo upload (encrypted)",
    "audit log wired to every route",
    "daily reminder schedule",
  ]

  # Compliance controls the scaffold satisfies on day one.
  prewired = [
    "phi-encryption",
    "audit-log",
    "ai-allowlist",
    "dependency-scan",
    "synthetic-only",
  ]

  # Gates that must be green before promotion to the prod pool.
  gates = [
    "phi-encryption",
    "audit-log",
    "ai-allowlist",
    "dependency-scan",
    "auto-logoff",
    "synthetic-only",
  ]

  synthetic_dataset = "post-op demo (12 pts)"
  input_capabilities = ["local-image-description"]
}
