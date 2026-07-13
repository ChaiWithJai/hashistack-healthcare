pack "rpm-wearables" {
  name = "RPM wearables"
  description = "Synthetic wearable observations streamed to a reviewed monitoring queue."
  profile = "stream"
  tier = 3
  wave = 3
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  treatment_recipes = ["guided-worklist", "event-timeline", "focused-task"]
  scaffold = ["SSE observation stream", "device filter", "threshold review queue", "session audit"]
  prewired = ["phi-encryption", "audit-log", "dependency-scan", "synthetic-only"]
  gates = ["phi-encryption", "audit-log", "dependency-scan", "auto-logoff", "synthetic-only", "escalation-path", "human-review", "access-roles"]
  synthetic_dataset = "wearable observations demo"
}
