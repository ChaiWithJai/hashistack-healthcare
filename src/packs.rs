//! Pack registry: serves signed use case packs.
//!
//! A pack is a declarative HCL manifest — the platform's extension unit,
//! mirroring Terraform modules and Nomad job specs. The registry only loads
//! manifests carrying a known signature chain; unsigned packs never list.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Signature roots the registry accepts. Phase 0: the platform key only.
/// Open question 2 in the RFC — clinician identity in the chain — lands here.
const TRUSTED_SIGNERS: &[&str] = &["platform-root-v1"];

/// Model tiers a pack may route an agent operation to. Serde rejects any
/// other name, so a typoed tier fails at registry load — same loud path as
/// the signature check, never a silent default at request time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingTier {
    /// Deterministic keyword rules — the offline/CI floor.
    Rules,
    /// OpenAI-compatible endpoint inside our VPC (`LOCAL_MODEL_URL`).
    Local,
    /// Frontier model under BAA (`FRONTIER_MODEL_URL`; stubbed in Phase 0).
    Frontier,
}

impl fmt::Display for RoutingTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RoutingTier::Rules => "rules",
            RoutingTier::Local => "local",
            RoutingTier::Frontier => "frontier",
        })
    }
}

/// Failure classes a pack may name in `escalate_on`. Decision 0001:
/// escalation is automatic and invisible to the doctor, but *which failures
/// may spend frontier tokens* is pack policy, reviewed and signed like the
/// gate list. The ladder (platform code) stays the outer authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EscalationReason {
    /// The tier's edit would unwire a satisfied required gate.
    GateRegression,
    /// The tier's reply was not a well-formed, effective edit (or the call
    /// failed — unreachable endpoints degrade to no-op edits).
    InvalidEdit,
}

impl fmt::Display for EscalationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EscalationReason::GateRegression => "gate-regression",
            EscalationReason::InvalidEdit => "invalid-edit",
        })
    }
}

/// Per-pack routing policy: which tier tries each agent operation FIRST,
/// and which failure classes consent to a frontier escalation. Expressed in
/// pack.hcl as a plain object attribute (`routing = { iterate = "local" }`)
/// — the attribute-object shape Packer/Waypoint use for `required_plugins`.
/// A pack that declares a `routing` object owns every field of it (absent
/// fields take the serde defaults below, escalate_on defaulting to *no*
/// consent); a pack that declares none inherits [`RoutingPolicy::default`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingPolicy {
    #[serde(default = "tier_frontier")]
    pub scaffold: RoutingTier,
    #[serde(default = "tier_local")]
    pub iterate: RoutingTier,
    #[serde(default = "tier_frontier")]
    pub review: RoutingTier,
    #[serde(default)]
    pub escalate_on: Vec<EscalationReason>,
}

impl RoutingPolicy {
    /// The tier that tries an operation first (decision 0001). Fix is
    /// deterministic platform code and always starts at rules.
    pub fn first_tier(&self, kind: crate::state::OpKind) -> RoutingTier {
        match kind {
            crate::state::OpKind::Scaffold => self.scaffold,
            crate::state::OpKind::Iterate => self.iterate,
            crate::state::OpKind::Fix => RoutingTier::Rules,
        }
    }
}

fn tier_frontier() -> RoutingTier {
    RoutingTier::Frontier
}

fn tier_local() -> RoutingTier {
    RoutingTier::Local
}

/// Platform defaults when a pack declares no routing (decision 0001): first
/// impressions and review stay on the frontier model, the chatty iterate
/// loop goes local, and both verifier failure classes may climb to frontier.
impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            scaffold: RoutingTier::Frontier,
            iterate: RoutingTier::Local,
            review: RoutingTier::Frontier,
            escalate_on: vec![
                EscalationReason::GateRegression,
                EscalationReason::InvalidEdit,
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackManifest {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub description: String,
    pub profile: String,
    pub tier: u8,
    pub wave: u8,
    pub signed_by: String,
    /// TODO(#5): demo scaffolds are feature strings. The pack spec calls for
    /// a scaffold/ directory holding a runnable hipaa-core app template, plus
    /// prompts/, policies/, gates/, synthetic/ (Synthea), and docs/ — the
    /// ejection payload for #11.
    pub scaffold: Vec<String>,
    pub prewired: Vec<String>,
    pub gates: Vec<String>,
    pub synthetic_dataset: String,
    /// Optional routing override. Lives inside the signed manifest, so where
    /// a model runs is reviewed and attested exactly like the gate list.
    /// Absent → platform defaults ([`RoutingPolicy::default`]).
    #[serde(default)]
    pub routing: Option<RoutingPolicy>,
}

impl PackManifest {
    /// The effective routing policy: the pack's own, or platform defaults.
    pub fn routing_policy(&self) -> RoutingPolicy {
        self.routing.clone().unwrap_or_default()
    }

    /// Citation for audit details — every routing decision names the policy
    /// that produced it, so the export answers "who decided" by itself.
    pub fn routing_source(&self) -> String {
        if self.routing.is_some() {
            format!("pack {} routing policy", self.id)
        } else {
            format!("platform default routing (pack {} declares none)", self.id)
        }
    }
}

#[derive(Deserialize)]
struct PackFile {
    pack: BTreeMap<String, PackManifest>,
}

/// Pack sources compiled into the binary so the registry and the git tree
/// can never disagree. The packs/ directory stays the single source of truth.
const PACK_SOURCES: &[&str] = &[
    include_str!("../packs/compliance-checklist/pack.hcl"),
    include_str!("../packs/hypertension-tracker/pack.hcl"),
    include_str!("../packs/patient-intake/pack.hcl"),
    include_str!("../packs/post-op-monitor/pack.hcl"),
    include_str!("../packs/insurance-verification/pack.hcl"),
];

pub fn parse_pack(source: &str) -> Result<PackManifest> {
    let file: PackFile = hcl::from_str(source).context("invalid pack.hcl")?;
    let (id, mut manifest) = file
        .pack
        .into_iter()
        .next()
        .context("pack.hcl declares no pack block")?;
    manifest.id = id;
    if !TRUSTED_SIGNERS.contains(&manifest.signed_by.as_str()) {
        bail!(
            "pack {} signed by untrusted key {:?} — refusing to register",
            manifest.id,
            manifest.signed_by
        );
    }
    Ok(manifest)
}

/// Load every built-in pack. Panics at startup on a bad manifest: a control
/// plane with a half-loaded registry is worse than one that fails loudly.
pub fn builtin_packs() -> Vec<PackManifest> {
    PACK_SOURCES
        .iter()
        .map(|src| parse_pack(src).expect("built-in pack manifest must parse and verify"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_parse_and_verify() {
        let packs = builtin_packs();
        assert_eq!(packs.len(), 5);
        assert!(packs.iter().any(|p| p.id == "post-op-monitor"));
        let iv = packs
            .iter()
            .find(|p| p.id == "insurance-verification")
            .unwrap();
        assert_eq!(iv.gates.len(), 9, "storyboard 1b promises nine checks");
    }

    #[test]
    fn routing_override_parses_and_defaults_apply_elsewhere() {
        let packs = builtin_packs();
        let iv = packs
            .iter()
            .find(|p| p.id == "insurance-verification")
            .unwrap();
        assert!(iv.routing.is_some(), "insurance-verification overrides");
        let policy = iv.routing_policy();
        assert_eq!(policy.scaffold, RoutingTier::Frontier);
        assert_eq!(policy.iterate, RoutingTier::Local);
        assert_eq!(policy.review, RoutingTier::Frontier);
        assert_eq!(
            policy.escalate_on,
            vec![
                EscalationReason::GateRegression,
                EscalationReason::InvalidEdit
            ]
        );
        assert!(iv.routing_source().contains("pack insurance-verification"));

        // Every other shipped pack declares nothing and gets the platform
        // defaults: scaffold/review frontier, iterate local, full consent.
        for p in packs.iter().filter(|p| p.id != "insurance-verification") {
            assert!(p.routing.is_none(), "{} should inherit defaults", p.id);
            assert_eq!(p.routing_policy(), RoutingPolicy::default());
            assert!(p.routing_source().contains("platform default"));
        }
    }

    #[test]
    fn unknown_routing_tier_is_refused_at_load() {
        let template = |routing: &str| {
            format!(
                r#"
                pack "typo" {{
                  name = "typo"
                  description = "bad routing policy"
                  profile = "web"
                  tier = 2
                  wave = 1
                  signed_by = "platform-root-v1"
                  scaffold = []
                  prewired = []
                  gates = []
                  synthetic_dataset = "none"
                  routing = {routing}
                }}
            "#
            )
        };
        // A typoed tier or escalation reason must fail registry load as
        // loudly as a bad signature — never a silent default at request time.
        assert!(parse_pack(&template(r#"{ iterate = "gpu-cluster" }"#)).is_err());
        assert!(parse_pack(&template(r#"{ escalate_on = ["vibes"] }"#)).is_err());
    }

    #[test]
    fn unsigned_pack_is_refused() {
        let rogue = r#"
            pack "rogue" {
              name = "rogue"
              description = "unsigned"
              profile = "web"
              tier = 2
              wave = 1
              signed_by = "somebody-else"
              scaffold = []
              prewired = []
              gates = []
              synthetic_dataset = "none"
            }
        "#;
        assert!(parse_pack(rogue).is_err());
    }
}
