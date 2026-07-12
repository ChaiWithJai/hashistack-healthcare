# Vault policy fragment — pack post-op-monitor (RFC 0001 §use-case-packs).
#
# Rendered per tenant at promotion and appended to the platform's base
# tenant policy (vault/policies/tenant-app.hcl); TENANT is the tenant slug.
# Same two capabilities as every web-profile app — transit crypto and
# short-TTL database creds — listed here so the pack's needs are reviewed
# and signed with the pack, not implied by the platform.

# hipaa-core encryptField/decryptField for check-in fields AND wound photos.
# The scaffold's photo upload is an in-memory stub until this key is wired;
# the stub says so in its own source (scaffold/src/main.rs, PhotoStub).
path "transit/encrypt/tenant-TENANT" {
  capabilities = ["update"]
}

path "transit/decrypt/tenant-TENANT" {
  capabilities = ["update"]
}

# Dynamic Postgres credentials, 1h TTL, auto-revoked with the allocation.
path "database/creds/tenant-TENANT" {
  capabilities = ["read"]
}
