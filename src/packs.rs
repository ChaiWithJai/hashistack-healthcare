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
    /// Scaffold feature strings — still real: they drive the demo UI's
    /// generate animation and become the app's initial feature list.
    /// post-op-monitor additionally ships the full RFC 0001 folder spec
    /// (scaffold/ as a runnable axum crate, prompts/, policies/, gates/,
    /// synthetic/, docs/) — the #5 pattern the other packs follow.
    /// TODO(#5), still pending: converting the other four packs,
    /// Synthea-generated synthetic data, and a registry signature covering
    /// the whole pack folder rather than the manifest alone.
    pub scaffold: Vec<String>,
    /// Relative path (inside the pack directory) of a runnable scaffold
    /// crate, when the pack ships one. `None` → feature strings only; the
    /// ejection bundle keeps its placeholder runtime and says so.
    #[serde(default)]
    pub scaffold_path: Option<String>,
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

/// One embedded pack file: (path relative to the pack directory, content).
pub type PackSourceFile = (&'static str, &'static str);

/// Runnable-scaffold sources, embedded at compile time exactly like
/// [`PACK_SOURCES`] so the ejection bundle and the git tree can never
/// disagree. Only packs converted to the full folder spec (issue #5) appear
/// here; the `scaffold/` prefix is what the ejection service remaps to
/// `app/`, and the synthetic seed rides along at the pack-relative path the
/// scaffold's `include_str!` and runtime loader both expect.
const POST_OP_MONITOR_SCAFFOLD: &[PackSourceFile] = &[
    (
        "scaffold/Cargo.toml",
        include_str!("../packs/post-op-monitor/scaffold/Cargo.toml"),
    ),
    (
        "scaffold/src/main.rs",
        include_str!("../packs/post-op-monitor/scaffold/src/main.rs"),
    ),
    (
        "synthetic/post-op-demo.json",
        include_str!("../packs/post-op-monitor/synthetic/post-op-demo.json"),
    ),
];

/// The embedded scaffold sources for a pack, if it ships a runnable
/// scaffold. Kept in lockstep with each manifest's `scaffold_path` — a test
/// below fails the build if the two ever disagree.
pub fn scaffold_sources(pack_id: &str) -> Option<&'static [PackSourceFile]> {
    match pack_id {
        "post-op-monitor" => Some(POST_OP_MONITOR_SCAFFOLD),
        _ => None,
    }
}

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
    fn scaffold_path_and_embedded_sources_agree_for_every_pack() {
        for pack in builtin_packs() {
            match (&pack.scaffold_path, scaffold_sources(&pack.id)) {
                (Some(path), Some(files)) => {
                    assert_eq!(path, "scaffold", "{}: spec fixes the folder name", pack.id);
                    assert!(
                        files.iter().any(|(p, _)| *p == "scaffold/src/main.rs"),
                        "{}: a runnable scaffold has source",
                        pack.id
                    );
                    assert!(
                        files.iter().any(|(p, _)| *p == "scaffold/Cargo.toml"),
                        "{}: a runnable scaffold has a manifest",
                        pack.id
                    );
                }
                (None, None) => {} // not yet converted (#5) — honestly absent
                (path, files) => panic!(
                    "{}: scaffold_path ({path:?}) and embedded sources ({}) disagree",
                    pack.id,
                    files.map(|f| f.len()).unwrap_or(0)
                ),
            }
        }
        // The pattern-setter is converted; this flips per pack as #5 lands.
        let post_op = builtin_packs()
            .into_iter()
            .find(|p| p.id == "post-op-monitor")
            .unwrap();
        assert_eq!(post_op.scaffold_path.as_deref(), Some("scaffold"));
    }

    #[test]
    fn post_op_synthetic_seed_is_marked_synthetic() {
        let (_, seed) = POST_OP_MONITOR_SCAFFOLD
            .iter()
            .find(|(p, _)| *p == "synthetic/post-op-demo.json")
            .expect("seed ships with the scaffold");
        assert!(
            seed.contains("SYNTHETIC DATA — generated, not derived from any real person"),
            "the seed must carry the synthetic notice the scaffold refuses to boot without"
        );
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
