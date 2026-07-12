pack "airgapped-support" {
  name = "air-gapped support"
  description = "Provides an offline reference search and support queue for disconnected clinical environments."
  profile = "local"
  tier = 2
  wave = 4
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  scaffold = ["offline reference index", "support queue", "removable-media export"]
  prewired = ["audit-log", "dependency-scan", "synthetic-only"]
  gates = ["audit-log", "dependency-scan", "synthetic-only", "human-review"]
  synthetic_dataset = "fictional offline support articles and tickets"
}
