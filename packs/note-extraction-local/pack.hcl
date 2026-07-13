pack "note-extraction-local" {
  name = "local note extraction"
  description = "Extracts reviewable administrative fields from notes without sending text off-device."
  profile = "local"
  tier = 2
  wave = 4
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  treatment_recipes = ["guided-worklist", "event-timeline", "focused-task"]
  scaffold = ["local note workspace", "structured draft", "human confirmation"]
  prewired = ["audit-log", "dependency-scan", "synthetic-only"]
  gates = ["audit-log", "dependency-scan", "synthetic-only", "human-review"]
  synthetic_dataset = "fictional notes and expected draft fields"
}
