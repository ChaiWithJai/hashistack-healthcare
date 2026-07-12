pack "clinical-dashboard" {
  name = "clinical dashboard"
  description = "Descriptive synthetic operations metrics with service-line filtering."
  profile = "web"
  tier = 3
  wave = 1
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  scaffold = ["metric cards", "service filter", "encounter table", "demo staff authentication"]
  prewired = ["audit-log", "synthetic-only", "access-roles"]
  gates = ["audit-log", "auto-logoff", "synthetic-only", "access-roles"]
  synthetic_dataset = "dashboard demo encounters"
}
