pack "ambient-scribe" {
  name = "ambient scribe"
  description = "Synthetic ambient transcript stream producing an unsigned clinician-editable draft."
  profile = "stream"
  tier = 3
  wave = 3
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  treatment_recipes = ["guided-worklist", "event-timeline", "focused-task"]
  scaffold = ["SSE ambient stream", "consent state", "SOAP draft", "sign-off refusal"]
  prewired = ["phi-encryption", "audit-log", "ai-allowlist", "dependency-scan", "synthetic-only"]
  gates = ["phi-encryption", "audit-log", "ai-allowlist", "dependency-scan", "auto-logoff", "synthetic-only", "human-review", "access-roles"]
  synthetic_dataset = "ambient scribe demo"
}
