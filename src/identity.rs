//! Identity registry (#10): who is calling, for which practice, as what role.
//!
//! Principals are declared in an `identities.hcl` (staging/identities.hcl,
//! parsed with hcl-rs exactly like packs) — id, display name, role, tenant,
//! and a bearer token. Every `/api` route resolves its caller through this
//! registry; the resolved [`Principal`] drives tenancy scoping, the role
//! capability check, audit attribution, and the authenticated co-sign record.
//!
//! # Phase 0 honesty: static tokens are the dev credential
//!
//! The `token` attribute is a static bearer string — the Phase 0 dev
//! credential, same spirit as `VAULT_TOKEN=staging-root`. **OIDC replaces
//! the token source, not the model**: when it lands, an issuer-verified
//! id_token maps to the same principal shape (id, name, role, tenant) and
//! every enforcement built here — tenant 404s, role 403s, session idle,
//! attestation binding — is untouched. NPI-verified clinician identity (RFC
//! open question 2) upgrades how a principal is *proven*, not what one *is*.
//!
//! # Two modes, declared honestly
//!
//! - **Dev** (no `IDENTITIES_FILE`): the embedded copy of
//!   staging/identities.hcl applies, and a request with NO Authorization
//!   header falls back to `dr-osei` so the zero-config doctor UI keeps
//!   working — audited as `auth.dev_fallback` on first use per boot, so the
//!   trail confesses the convenience. A *present but unknown* token is still
//!   401 even in dev.
//! - **Strict** (`IDENTITIES_FILE=path`): missing or invalid tokens answer
//!   401. Staging boots this way (scripts/staging-up.sh).
//!
//! # Sessions: the platform honors its own auto-logoff gate
//!
//! With `SESSION_IDLE_SECS` set (staging default; off in dev), a token idle
//! past the limit answers 401 with an `auth.session_expired` audit event —
//! the same auto-logoff the gate engine demands of every generated app,
//! applied to the platform itself. Kept deliberately simple: an in-memory
//! last-seen timestamp per token. With Phase 0 static tokens the denied
//! request is the logoff boundary (the next request re-authenticates and
//! starts a fresh session); an OIDC credential carries its own expiry and
//! makes the 401 terminal.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::state::now_unix;

/// The embedded dev registry — the same file staging points
/// `IDENTITIES_FILE` at, compile-time included so the dev default and the
/// git tree can never disagree (the PACK_SOURCES pattern).
const DEV_IDENTITIES: &str = include_str!("../staging/identities.hcl");

/// The principal the dev registry falls back to on a missing Authorization
/// header — the demo doctor. Only ever active with no `IDENTITIES_FILE`.
const DEV_FALLBACK_ID: &str = "dr-osei";

/// Closed role set. Serde rejects any other name, so a typoed role fails
/// registry load as loudly as an unsigned pack — never a silent default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Practices medicine: everything in their tenant, including the
    /// co-sign — releasing an app to real patients is a clinical act.
    Clinician,
    /// Runs the practice: builds and operates in their tenant, but may not
    /// promote/co-sign a release or export the platform-wide audit stream.
    Staff,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Clinician => "clinician",
            Role::Staff => "staff",
        }
    }

    /// The capability check — one place, not scattered ifs. Clinicians hold
    /// every capability in their tenant; staff are denied the release and
    /// platform-audit capabilities (403 `auth.role_denied`).
    pub fn allows(&self, capability: Capability) -> bool {
        match self {
            Role::Clinician => true,
            Role::Staff => match capability {
                Capability::CoSignRelease | Capability::ExportPlatformAudit => false,
            },
        }
    }
}

/// Capabilities that are role-gated beyond tenancy. Everything not listed
/// here (describe, iterate, fix, review, rollback, operate, app export,
/// app-scoped audit) is tenant-scoped but role-open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// Promote to the prod pool / co-sign the release attestation.
    CoSignRelease,
    /// Export the platform-wide (cross-tenant, HMAC-form) audit stream.
    ExportPlatformAudit,
}

impl Capability {
    /// Human phrasing for 403 bodies and `auth.role_denied` audit details.
    pub fn describe(&self) -> &'static str {
        match self {
            Capability::CoSignRelease => "promote or co-sign a release",
            Capability::ExportPlatformAudit => "export the platform-wide audit stream",
        }
    }
}

/// A resolved caller. Cloned into the request extensions by the auth
/// middleware; the bearer token never serializes.
#[derive(Clone, Debug, Serialize)]
pub struct Principal {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub tenant: String,
    /// The Phase 0 dev credential (see module doc). Never serialized.
    #[serde(skip)]
    pub token: String,
}

/// One `identity "<id>" { ... }` block. `deny_unknown_fields` keeps the
/// declared schema honest — an unexpected attribute fails the load.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipalDecl {
    name: String,
    role: Role,
    tenant: String,
    token: String,
}

#[derive(Deserialize)]
struct IdentityFile {
    identity: BTreeMap<String, PrincipalDecl>,
}

/// Tenant names cross several trust boundaries: Vault paths, Nomad
/// namespaces, database identifiers, and hostnames. Keep one deliberately
/// conservative grammar before any of those renderers see the value.
pub fn validate_tenant_slug(value: &str) -> Result<()> {
    validate_slug("tenant", value, 40)
}

/// DNS/HCL-safe application identifier validation used by the deploy
/// renderer. App ids are allowed the full DNS-label length.
pub fn validate_app_slug(value: &str) -> Result<()> {
    validate_slug("app id", value, 63)
}

fn validate_slug(kind: &str, value: &str, max: usize) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > max {
        bail!("{kind} must be 1..={max} ASCII bytes");
    }
    if !bytes[0].is_ascii_lowercase() {
        bail!("{kind} must start with a lowercase ASCII letter");
    }
    if bytes.last() == Some(&b'-') {
        bail!("{kind} must not end with a hyphen");
    }
    let mut previous_hyphen = false;
    for byte in bytes {
        let valid = byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-';
        if !valid {
            bail!("{kind} contains an unsafe character");
        }
        if *byte == b'-' && previous_hyphen {
            bail!("{kind} must not contain consecutive hyphens");
        }
        previous_hyphen = *byte == b'-';
    }
    Ok(())
}

/// The loaded registry: principals, the (dev-only) fallback, and the
/// in-memory session table behind `SESSION_IDLE_SECS`.
#[derive(Debug)]
pub struct Registry {
    principals: Vec<Principal>,
    /// `Some(principal id)` only in dev mode (no `IDENTITIES_FILE`).
    fallback_id: Option<String>,
    /// The `auth.dev_fallback` confession fires once per boot.
    fallback_announced: AtomicBool,
    /// `None` = idle expiry off (the dev default).
    idle_secs: Option<u64>,
    /// Last-seen unix seconds per token — the whole session store.
    last_seen: Mutex<HashMap<String, u64>>,
    /// Where the principals came from, for the boot log.
    source: String,
}

impl Registry {
    /// Parse an identities.hcl. Loud on: no principals, an unknown role or
    /// attribute (serde), a blank/duplicate token, a blank tenant or name,
    /// or a `fallback` id that is not declared.
    pub fn parse(source: &str, fallback: Option<&str>, idle_secs: Option<u64>) -> Result<Self> {
        let file: IdentityFile = hcl::from_str(source).context("invalid identities.hcl")?;
        if file.identity.is_empty() {
            bail!("identities.hcl declares no identity blocks");
        }
        let mut principals = Vec::new();
        let mut seen_tokens: BTreeMap<&str, &str> = BTreeMap::new();
        for (id, decl) in &file.identity {
            let token = decl.token.trim();
            if token.is_empty() {
                bail!("identity {id:?} declares an empty token");
            }
            if let Some(other) = seen_tokens.insert(token, id) {
                bail!("identities {other:?} and {id:?} share a bearer token — refusing an ambiguous registry");
            }
            if decl.tenant.trim().is_empty() || decl.name.trim().is_empty() {
                bail!("identity {id:?} needs a non-empty name and tenant");
            }
            validate_tenant_slug(decl.tenant.trim())
                .with_context(|| format!("identity {id:?} declares an unsafe tenant"))?;
            principals.push(Principal {
                id: id.clone(),
                name: decl.name.trim().to_string(),
                role: decl.role,
                tenant: decl.tenant.trim().to_string(),
                token: token.to_string(),
            });
        }
        let fallback_id = match fallback {
            None => None,
            Some(id) => {
                if !principals.iter().any(|p| p.id == id) {
                    bail!("fallback principal {id:?} is not declared in the registry");
                }
                Some(id.to_string())
            }
        };
        Ok(Self {
            principals,
            fallback_id,
            fallback_announced: AtomicBool::new(false),
            idle_secs: idle_secs.filter(|&n| n > 0),
            last_seen: Mutex::new(HashMap::new()),
            source: "inline".to_string(),
        })
    }

    /// The embedded dev registry: staging/identities.hcl compiled in, the
    /// dr-osei fallback on, idle expiry off. Panics on a bad embedded file —
    /// same loud-boot rule as `packs::builtin_packs`.
    pub fn dev_default() -> Self {
        let mut registry = Self::parse(DEV_IDENTITIES, Some(DEV_FALLBACK_ID), None)
            .expect("embedded staging/identities.hcl must parse");
        registry.source = "embedded dev registry (no IDENTITIES_FILE)".to_string();
        registry
    }

    /// Boot-time construction (src/lib.rs): `IDENTITIES_FILE` set → strict
    /// mode from that file (no fallback); unset → the embedded dev default.
    /// `SESSION_IDLE_SECS` (integer seconds; 0/unset = off) applies to both.
    pub fn from_env() -> Result<Self> {
        let idle_secs = std::env::var("SESSION_IDLE_SECS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                s.trim().parse::<u64>().with_context(|| {
                    format!("SESSION_IDLE_SECS={s:?} is not a whole number of seconds")
                })
            })
            .transpose()?;
        match std::env::var("IDENTITIES_FILE")
            .ok()
            .filter(|p| !p.trim().is_empty())
        {
            Some(path) => {
                let source = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading IDENTITIES_FILE at {path}"))?;
                let mut registry = Self::parse(&source, None, idle_secs)
                    .with_context(|| format!("loading IDENTITIES_FILE at {path}"))?;
                registry.source = path;
                Ok(registry)
            }
            None => {
                let mut registry = Self::dev_default();
                registry.idle_secs = idle_secs.filter(|&n| n > 0);
                Ok(registry)
            }
        }
    }

    pub fn by_token(&self, token: &str) -> Option<&Principal> {
        self.principals.iter().find(|p| p.token == token)
    }

    pub fn by_id(&self, id: &str) -> Option<&Principal> {
        self.principals.iter().find(|principal| principal.id == id)
    }

    /// The dev-mode fallback for a MISSING Authorization header. `None` in
    /// strict mode — the caller answers 401.
    pub fn fallback(&self) -> Option<&Principal> {
        let id = self.fallback_id.as_deref()?;
        self.principals.iter().find(|p| p.id == id)
    }

    /// First-use-per-boot latch for the `auth.dev_fallback` audit event.
    /// Returns true exactly once.
    pub fn announce_dev_fallback(&self) -> bool {
        !self.fallback_announced.swap(true, Ordering::SeqCst)
    }

    pub fn idle_secs(&self) -> Option<u64> {
        self.idle_secs
    }

    pub fn principal_count(&self) -> usize {
        self.principals.len()
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Record a token use at `now`. `Err(idle_secs)` when the session sat
    /// idle past the limit: the entry is dropped (the denied request IS the
    /// logoff; the next request starts a fresh session — see module doc).
    pub fn touch_at(&self, token: &str, now: u64) -> std::result::Result<(), u64> {
        let Some(idle) = self.idle_secs else {
            return Ok(());
        };
        let mut seen = self.last_seen.lock().unwrap();
        match seen.get(token) {
            Some(&last) if now.saturating_sub(last) > idle => {
                seen.remove(token);
                Err(idle)
            }
            _ => {
                seen.insert(token.to_string(), now);
                Ok(())
            }
        }
    }

    /// [`Registry::touch_at`] at the current time — the middleware path.
    pub fn touch(&self, token: &str) -> std::result::Result<(), u64> {
        self.touch_at(token, now_unix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_dev_registry_parses_with_two_tenants_and_a_staff_role() {
        let registry = Registry::dev_default();
        assert_eq!(registry.principal_count(), 3);
        let osei = registry.by_token("dev-token-osei").unwrap();
        assert_eq!(
            (osei.id.as_str(), osei.tenant.as_str()),
            ("dr-osei", "meridian")
        );
        assert_eq!(osei.role, Role::Clinician);
        assert_eq!(osei.name, "Dr. A. Osei");
        let park = registry.by_token("dev-token-park").unwrap();
        assert_eq!(park.tenant, "lakeside");
        assert_eq!(park.role, Role::Clinician);
        let staff = registry.by_token("dev-token-rivera").unwrap();
        assert_eq!(
            (staff.role, staff.tenant.as_str()),
            (Role::Staff, "meridian")
        );
        assert_eq!(registry.fallback().unwrap().id, "dr-osei");
        assert!(registry.idle_secs().is_none(), "idle expiry is off in dev");
    }

    /// Build one `identity` block with the given role/tenant/token/extra.
    fn block(id: &str, role: &str, tenant: &str, token: &str, extra: &str) -> String {
        format!(
            "identity \"{id}\" {{\n  name = \"N\"\n  role = \"{role}\"\n  tenant = \"{tenant}\"\n  token = \"{token}\"\n{extra}}}\n"
        )
    }

    #[test]
    fn unknown_role_fails_the_load_loudly() {
        let err = Registry::parse(&block("x", "superadmin", "t", "tk", ""), None, None)
            .expect_err("an undeclared role must be refused");
        assert!(err.to_string().contains("identities.hcl"), "{err:#}");
    }

    #[test]
    fn unknown_attribute_duplicate_token_and_blank_fields_are_refused() {
        assert!(
            Registry::parse(
                &block("x", "staff", "t", "tk", "  npi = \"1\"\n"),
                None,
                None
            )
            .is_err(),
            "deny_unknown_fields"
        );
        let two = format!(
            "{}{}",
            block("a", "staff", "t", "same", ""),
            block("b", "staff", "t", "same", "")
        );
        let dup = Registry::parse(&two, None, None).expect_err("shared tokens are ambiguous");
        assert!(dup.to_string().contains("share a bearer token"), "{dup:#}");
        assert!(Registry::parse(&block("x", "staff", "", "tk", ""), None, None).is_err());
        assert!(Registry::parse(&block("x", "staff", "t", " ", ""), None, None).is_err());
        assert!(Registry::parse("", None, None).is_err(), "empty registry");
    }

    #[test]
    fn tenant_slugs_are_safe_for_vault_nomad_database_and_dns_rendering() {
        for valid in [
            "meridian",
            "clinic-2",
            "a",
            "a234567890123456789012345678901234567890",
        ] {
            assert!(validate_tenant_slug(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "Meridian",
            "clinic_2",
            "clinic.example",
            "../vault",
            "two--hyphens",
            "trailing-",
            "${meta.role}",
            "tenant\"}\njob \"owned\" {",
            "méridian",
            "a2345678901234567890123456789012345678901",
        ] {
            assert!(validate_tenant_slug(invalid).is_err(), "{invalid:?}");
            let source = block("x", "staff", invalid, "tk", "");
            assert!(
                Registry::parse(&source, None, None).is_err(),
                "registry accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn staff_capability_check_denies_release_and_platform_audit_only() {
        assert!(Role::Clinician.allows(Capability::CoSignRelease));
        assert!(Role::Clinician.allows(Capability::ExportPlatformAudit));
        assert!(!Role::Staff.allows(Capability::CoSignRelease));
        assert!(!Role::Staff.allows(Capability::ExportPlatformAudit));
    }

    #[test]
    fn session_idle_expires_and_the_denied_request_restarts_the_session() {
        let one = block("x", "clinician", "t", "tk", "");
        let registry = Registry::parse(&one, None, Some(30)).unwrap();
        assert_eq!(registry.touch_at("tk", 1_000), Ok(()));
        assert_eq!(
            registry.touch_at("tk", 1_030),
            Ok(()),
            "at the limit is not past it"
        );
        assert_eq!(
            registry.touch_at("tk", 1_061),
            Err(30),
            "idle past the limit"
        );
        // The 401 was the logoff boundary: the next use starts fresh.
        assert_eq!(registry.touch_at("tk", 1_062), Ok(()));
        // A registry without the env never expires anything.
        let no_idle = Registry::parse(&one, None, None).unwrap();
        assert_eq!(no_idle.touch_at("tk", 1), Ok(()));
        assert_eq!(no_idle.touch_at("tk", u64::MAX), Ok(()));
    }

    #[test]
    fn dev_fallback_announces_exactly_once_per_boot() {
        let registry = Registry::dev_default();
        assert!(registry.announce_dev_fallback());
        assert!(!registry.announce_dev_fallback());
    }

    #[test]
    fn strict_parse_has_no_fallback() {
        let registry = Registry::parse(DEV_IDENTITIES, None, None).unwrap();
        assert!(registry.fallback().is_none());
    }
}
