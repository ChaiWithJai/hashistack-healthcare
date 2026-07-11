# Identity registry (#10) — the Phase 0 principal declarations.
#
# HONEST LABELING: the `token` attribute is a static bearer token — the
# Phase 0 DEV CREDENTIAL, same spirit as VAULT_TOKEN=staging-root. OIDC
# replaces the token SOURCE (an issuer-verified id_token instead of a
# declared string), not the model: principal id, display name, role, and
# tenant — and everything the control plane enforces with them — stay
# exactly as declared here. Never put a production credential in this file.
#
# This file doubles as the embedded dev default (compile-time include in
# src/identity.rs, the PACK_SOURCES pattern): with no IDENTITIES_FILE env
# set, these principals exist and a request with NO Authorization header
# falls back to dr-osei — audited as `auth.dev_fallback`. With
# IDENTITIES_FILE set (staging), missing or invalid tokens answer 401.
#
# Roles are a closed set (clinician | staff) — an unknown role fails the
# load loudly, like an unsigned pack.

identity "dr-osei" {
  name   = "Dr. A. Osei"
  role   = "clinician"
  tenant = "meridian"
  token  = "dev-token-osei"
}

identity "dr-park" {
  name   = "Dr. J. Park"
  role   = "clinician"
  tenant = "lakeside"
  token  = "dev-token-park"
}

identity "ms-rivera" {
  name   = "M. Rivera"
  role   = "staff"
  tenant = "meridian"
  token  = "dev-token-rivera"
}
