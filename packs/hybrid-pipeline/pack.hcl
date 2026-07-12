pack "hybrid-pipeline" {
  name = "hybrid local pipeline"
  description = "Processes sensitive text locally and prepares a de-identified payload for optional reviewed cloud processing."
  profile = "local"
  tier = 3
  wave = 4
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  scaffold = ["local redaction stage", "disclosure preview", "explicit release approval"]
  prewired = ["audit-log", "ai-allowlist", "dependency-scan", "synthetic-only"]
  gates = ["audit-log", "ai-allowlist", "dependency-scan", "synthetic-only", "human-review", "access-roles"]
  synthetic_dataset = "fictional notes and approved disclosure payloads"
}
