pack "deid-local" {
  name = "local de-identification"
  description = "Finds and replaces direct identifiers in text entirely on the clinician's device."
  profile = "local"
  tier = 2
  wave = 4
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  scaffold = ["local text workspace", "identifier review", "export without source retention"]
  prewired = ["audit-log", "dependency-scan", "synthetic-only"]
  gates = ["audit-log", "dependency-scan", "synthetic-only", "human-review"]
  synthetic_dataset = "fictional clinical notes with invented identifiers"
}
