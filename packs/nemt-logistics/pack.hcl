pack "nemt-logistics" {
  name = "NEMT logistics"
  description = "Coordinates non-emergency medical transportation rides, status updates, and staff escalation."
  profile = "web"
  tier = 3
  wave = 2
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  treatment_recipes = ["guided-worklist", "event-timeline", "focused-task"]
  scaffold = ["ride coordination queue", "status updates", "missed-ride escalation", "audit log wired to every route"]
  prewired = ["phi-encryption", "audit-log", "dependency-scan", "synthetic-only"]
  gates = ["phi-encryption", "audit-log", "dependency-scan", "synthetic-only", "auto-logoff", "access-roles", "escalation-path", "human-review"]
  synthetic_dataset = "fictional rides and transport providers"
}
