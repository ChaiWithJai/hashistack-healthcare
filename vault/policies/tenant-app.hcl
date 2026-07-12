# Vault policy template for one promoted tenant app allocation.
# Rendered per tenant by the deploy service (TENANT is the tenant slug) and
# mounted at sys/policies/acl/tenant-TENANT on the tenant's first promote
# (#9). Read it back: vault policy read tenant-TENANT.
#
# Two capabilities only. The app can encrypt/decrypt its own tenant's fields
# and mint short-TTL database credentials. It cannot read another tenant's
# keys, list mounts, or touch its own policy.
#
# Honesty notes (#9): staging shares one database role (tenant-app) — the
# per-tenant DB role is a Phase 1 (cloud) item. And in dev-mode staging the
# control plane holds the root token, so this policy exists and names the
# exact live paths but is not yet the enforcing credential; per-allocation
# tokens bound to it are the Phase 1 item alongside Vault workload identity.

# hipaa-core encryptField/decryptField — keys never leave Vault.
path "transit/encrypt/tenant-TENANT" {
  capabilities = ["update"]
}

path "transit/decrypt/tenant-TENANT" {
  capabilities = ["update"]
}

# Dynamic Postgres credentials, 1h TTL, revoked with the allocation.
path "database/creds/tenant-app" {
  capabilities = ["read"]
}
