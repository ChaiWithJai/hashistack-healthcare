pack "inbound-scheduling" {
  name = "inbound scheduling"
  description = "Collects inbound appointment requests, offers administrative availability, and routes exceptions to staff."
  profile = "web"
  tier = 3
  wave = 2
  signed_by = "platform-root-v1"
  scaffold_path = "scaffold"
  quality_contract = "artifact-quality.json"
  treatment_recipes = ["guided-worklist", "event-timeline", "focused-task"]
  scaffold = ["appointment request inbox", "availability matching", "manual escalation", "audit log wired to every route"]
  prewired = ["phi-encryption", "audit-log", "dependency-scan", "synthetic-only"]
  gates = ["phi-encryption", "audit-log", "dependency-scan", "synthetic-only", "auto-logoff", "access-roles", "escalation-path", "human-review"]
  synthetic_dataset = "fictional appointment requests and availability"
}
