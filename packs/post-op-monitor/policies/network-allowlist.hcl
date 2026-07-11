# Network allowlist — pack post-op-monitor.
#
# The BAA'd endpoints an app scaffolded from this pack may call. The
# ai-allowlist gate (src/gates.rs ENDPOINT_ALLOWLIST) fails promotion on
# any external call not covered here; an un-BAA'd AI endpoint is the single
# most common way a vibe-coded tool leaks PHI. Declared with the pack so
# widening the app's reach is a signed, reviewed change.

allowlist "post-op-monitor" {
  # Platform-internal — never leaves the VPC.
  endpoints = [
    "vault.internal",    # transit encrypt/decrypt, db creds
    "postgres.internal", # tenant database
  ]

  # External, BAA-covered.
  baa_endpoints = [
    "api.anthropic.com", # platform LLM key, scoped per environment, under BAA
  ]

  # Deliberately absent: patient-notification vendors (SMS/email reminders
  # for the "daily reminder schedule" feature). Adding one is a pack.hcl
  # revision plus a signed BAA — not an app-level edit.
}
