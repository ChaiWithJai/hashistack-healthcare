pack "visit-notes" {
  name = "visit notes"
  description = "Synthetic transcript segments assembled into a clinician-reviewed visit-note draft."
  profile = "stream"
  tier = 3
  wave = 3
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  scaffold = ["SSE transcript stream", "speaker timeline", "draft note", "clinician review"]
  prewired = ["phi-encryption", "audit-log", "ai-allowlist", "dependency-scan", "synthetic-only"]
  gates = ["phi-encryption", "audit-log", "ai-allowlist", "dependency-scan", "auto-logoff", "synthetic-only", "human-review", "access-roles"]
  synthetic_dataset = "visit transcript demo"
  input_capabilities = ["local-audio-transcription"]
}
