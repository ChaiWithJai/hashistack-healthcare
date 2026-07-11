# Vault policy template for one promoted tenant app allocation.
# Rendered per tenant by the deploy service; TENANT is the tenant slug.
#
# Two capabilities only. The app can encrypt/decrypt its own tenant's fields
# and mint short-TTL database credentials for its own database. It cannot
# read another tenant's keys, list mounts, or touch its own policy.

# hipaa-core encryptField/decryptField — keys never leave Vault.
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
