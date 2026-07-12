pack "patient-portal" {
  name = "patient portal"
  description = "Scoped synthetic records, appointments, and secure-message learning workflow."
  profile = "web"
  tier = 3
  wave = 1
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  scaffold = ["patient record view", "appointment list", "role-scoped messages", "explicit demo authentication"]
  prewired = ["audit-log", "synthetic-only", "access-roles"]
  gates = ["audit-log", "auto-logoff", "synthetic-only", "access-roles"]
  synthetic_dataset = "portal demo (2 pts)"
}
