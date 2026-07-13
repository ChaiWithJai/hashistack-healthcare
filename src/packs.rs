//! Pack registry: serves signed use case packs.
//!
//! A pack is a declarative HCL manifest — the platform's extension unit,
//! mirroring Terraform modules and Nomad job specs. The registry only loads
//! manifests carrying a known signature chain; unsigned packs never list.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

/// Signature roots the registry accepts. Phase 0: the platform key only.
/// Open question 2 in the RFC — clinician identity in the chain — lands here.
const TRUSTED_SIGNERS: &[&str] = &["platform-root-v1"];

/// Historical routing values retained so existing signed packs still parse.
/// Production resolves every application edit to deterministic rules. Gemma
/// treatment planning uses the separate workspace provider boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingTier {
    /// Deterministic keyword rules — the offline/CI floor.
    Rules,
    /// Historical value. Production resolves it to rules.
    Local,
    /// Historical value. Production resolves it to rules.
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
    /// An exported, practice-owned starter names the trusted built-in pack
    /// whose gate policy and clinical profile remain authoritative. Built-in
    /// packs leave this empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    /// Scaffold feature strings — still real: they drive the demo UI's
    /// generate animation and become the app's initial feature list.
    /// Every built-in pack ships a runnable axum scaffold plus a synthetic
    /// fixture and an executable artifact-quality contract. Still pending:
    /// Synthea-generated fixtures and a registry signature covering the
    /// whole pack folder rather than the manifest alone.
    pub scaffold: Vec<String>,
    /// Relative path (inside the pack directory) of a runnable scaffold
    /// crate, when the pack ships one. `None` → feature strings only; the
    /// ejection bundle keeps its placeholder runtime and says so.
    #[serde(default)]
    pub scaffold_path: Option<String>,
    /// Pack-owned executable quality contract copied into every ejected
    /// repository and interpreted by the generic artifact eval harness.
    #[serde(default)]
    pub quality_contract: Option<String>,
    /// Whether the pack's scaffold follows the source annotations consumed
    /// by the Phase 0 static gate inspectors. Runnable does not imply that
    /// every compliance verdict can be inferred from source.
    #[serde(default)]
    pub static_evidence: bool,
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
    include_str!("../packs/patient-portal/pack.hcl"),
    include_str!("../packs/clinical-dashboard/pack.hcl"),
    include_str!("../packs/nemt-logistics/pack.hcl"),
    include_str!("../packs/inbound-scheduling/pack.hcl"),
    include_str!("../packs/outbound-followup/pack.hcl"),
    include_str!("../packs/rpm-wearables/pack.hcl"),
    include_str!("../packs/visit-notes/pack.hcl"),
    include_str!("../packs/ambient-scribe/pack.hcl"),
    include_str!("../packs/deid-local/pack.hcl"),
    include_str!("../packs/note-extraction-local/pack.hcl"),
    include_str!("../packs/airgapped-support/pack.hcl"),
    include_str!("../packs/hybrid-pipeline/pack.hcl"),
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
        "scaffold/Cargo.lock",
        include_str!("../packs/post-op-monitor/scaffold/Cargo.lock"),
    ),
    (
        "scaffold/src/main.rs",
        include_str!("../packs/post-op-monitor/scaffold/src/main.rs"),
    ),
    (
        "synthetic/post-op-demo.json",
        include_str!("../packs/post-op-monitor/synthetic/post-op-demo.json"),
    ),
    (
        "artifact-quality.json",
        include_str!("../packs/post-op-monitor/artifact-quality.json"),
    ),
];

const HYPERTENSION_TRACKER_SCAFFOLD: &[PackSourceFile] = &[
    (
        "scaffold/Cargo.toml",
        include_str!("../packs/hypertension-tracker/scaffold/Cargo.toml"),
    ),
    (
        "scaffold/Cargo.lock",
        include_str!("../packs/hypertension-tracker/scaffold/Cargo.lock"),
    ),
    (
        "scaffold/src/main.rs",
        include_str!("../packs/hypertension-tracker/scaffold/src/main.rs"),
    ),
    (
        "synthetic/htn-demo.json",
        include_str!("../packs/hypertension-tracker/synthetic/htn-demo.json"),
    ),
    (
        "artifact-quality.json",
        include_str!("../packs/hypertension-tracker/artifact-quality.json"),
    ),
];

const PATIENT_INTAKE_SCAFFOLD: &[PackSourceFile] = &[
    (
        "scaffold/Cargo.toml",
        include_str!("../packs/patient-intake/scaffold/Cargo.toml"),
    ),
    (
        "scaffold/Cargo.lock",
        include_str!("../packs/patient-intake/scaffold/Cargo.lock"),
    ),
    (
        "scaffold/src/main.rs",
        include_str!("../packs/patient-intake/scaffold/src/main.rs"),
    ),
    (
        "synthetic/intake-demo.json",
        include_str!("../packs/patient-intake/synthetic/intake-demo.json"),
    ),
    (
        "artifact-quality.json",
        include_str!("../packs/patient-intake/artifact-quality.json"),
    ),
];

const INSURANCE_VERIFICATION_SCAFFOLD: &[PackSourceFile] = &[
    (
        "scaffold/Cargo.toml",
        include_str!("../packs/insurance-verification/scaffold/Cargo.toml"),
    ),
    (
        "scaffold/Cargo.lock",
        include_str!("../packs/insurance-verification/scaffold/Cargo.lock"),
    ),
    (
        "scaffold/src/main.rs",
        include_str!("../packs/insurance-verification/scaffold/src/main.rs"),
    ),
    (
        "synthetic/insurance-demo.json",
        include_str!("../packs/insurance-verification/synthetic/insurance-demo.json"),
    ),
    (
        "artifact-quality.json",
        include_str!("../packs/insurance-verification/artifact-quality.json"),
    ),
];

const COMPLIANCE_CHECKLIST_SCAFFOLD: &[PackSourceFile] = &[
    (
        "scaffold/Cargo.toml",
        include_str!("../packs/compliance-checklist/scaffold/Cargo.toml"),
    ),
    (
        "scaffold/Cargo.lock",
        include_str!("../packs/compliance-checklist/scaffold/Cargo.lock"),
    ),
    (
        "scaffold/src/main.rs",
        include_str!("../packs/compliance-checklist/scaffold/src/main.rs"),
    ),
    (
        "synthetic/compliance-demo.json",
        include_str!("../packs/compliance-checklist/synthetic/compliance-demo.json"),
    ),
    (
        "artifact-quality.json",
        include_str!("../packs/compliance-checklist/artifact-quality.json"),
    ),
];

macro_rules! pack_scaffold {
    ($name:ident, $dir:literal, $seed:literal) => {
        const $name: &[PackSourceFile] = &[
            (
                "scaffold/Cargo.toml",
                include_str!(concat!("../packs/", $dir, "/scaffold/Cargo.toml")),
            ),
            (
                "scaffold/Cargo.lock",
                include_str!(concat!("../packs/", $dir, "/scaffold/Cargo.lock")),
            ),
            (
                "scaffold/src/main.rs",
                include_str!(concat!("../packs/", $dir, "/scaffold/src/main.rs")),
            ),
            ($seed, include_str!(concat!("../packs/", $dir, "/", $seed))),
            (
                "artifact-quality.json",
                include_str!(concat!("../packs/", $dir, "/artifact-quality.json")),
            ),
        ];
    };
}

pack_scaffold!(
    PATIENT_PORTAL_SCAFFOLD,
    "patient-portal",
    "synthetic/portal-demo.json"
);
pack_scaffold!(
    CLINICAL_DASHBOARD_SCAFFOLD,
    "clinical-dashboard",
    "synthetic/dashboard-demo.json"
);
pack_scaffold!(
    NEMT_LOGISTICS_SCAFFOLD,
    "nemt-logistics",
    "synthetic/rides.json"
);
pack_scaffold!(
    INBOUND_SCHEDULING_SCAFFOLD,
    "inbound-scheduling",
    "synthetic/requests.json"
);
pack_scaffold!(
    OUTBOUND_FOLLOWUP_SCAFFOLD,
    "outbound-followup",
    "synthetic/outbound-followup-demo.json"
);
pack_scaffold!(
    RPM_WEARABLES_SCAFFOLD,
    "rpm-wearables",
    "synthetic/demo.json"
);
pack_scaffold!(VISIT_NOTES_SCAFFOLD, "visit-notes", "synthetic/demo.json");
pack_scaffold!(
    AMBIENT_SCRIBE_SCAFFOLD,
    "ambient-scribe",
    "synthetic/demo.json"
);
pack_scaffold!(DEID_LOCAL_SCAFFOLD, "deid-local", "synthetic/demo.json");
pack_scaffold!(
    NOTE_EXTRACTION_LOCAL_SCAFFOLD,
    "note-extraction-local",
    "synthetic/demo.json"
);
pack_scaffold!(
    AIRGAPPED_SUPPORT_SCAFFOLD,
    "airgapped-support",
    "synthetic/demo.json"
);
pack_scaffold!(
    HYBRID_PIPELINE_SCAFFOLD,
    "hybrid-pipeline",
    "synthetic/demo.json"
);

/// The embedded scaffold sources for a pack, if it ships a runnable
/// scaffold. Kept in lockstep with each manifest's `scaffold_path` — a test
/// below fails the build if the two ever disagree.
pub fn scaffold_sources(pack_id: &str) -> Option<&'static [PackSourceFile]> {
    match pack_id {
        "post-op-monitor" => Some(POST_OP_MONITOR_SCAFFOLD),
        "hypertension-tracker" => Some(HYPERTENSION_TRACKER_SCAFFOLD),
        "patient-intake" => Some(PATIENT_INTAKE_SCAFFOLD),
        "insurance-verification" => Some(INSURANCE_VERIFICATION_SCAFFOLD),
        "compliance-checklist" => Some(COMPLIANCE_CHECKLIST_SCAFFOLD),
        "patient-portal" => Some(PATIENT_PORTAL_SCAFFOLD),
        "clinical-dashboard" => Some(CLINICAL_DASHBOARD_SCAFFOLD),
        "nemt-logistics" => Some(NEMT_LOGISTICS_SCAFFOLD),
        "inbound-scheduling" => Some(INBOUND_SCHEDULING_SCAFFOLD),
        "outbound-followup" => Some(OUTBOUND_FOLLOWUP_SCAFFOLD),
        "rpm-wearables" => Some(RPM_WEARABLES_SCAFFOLD),
        "visit-notes" => Some(VISIT_NOTES_SCAFFOLD),
        "ambient-scribe" => Some(AMBIENT_SCRIBE_SCAFFOLD),
        "deid-local" => Some(DEID_LOCAL_SCAFFOLD),
        "note-extraction-local" => Some(NOTE_EXTRACTION_LOCAL_SCAFFOLD),
        "airgapped-support" => Some(AIRGAPPED_SUPPORT_SCAFFOLD),
        "hybrid-pipeline" => Some(HYBRID_PIPELINE_SCAFFOLD),
        _ => None,
    }
}

/// The pack's signed network policy, embedded like everything else the gate
/// engine consumes so the evidence (#3) and the git tree can never disagree.
const POST_OP_MONITOR_ALLOWLIST: &str =
    include_str!("../packs/post-op-monitor/policies/network-allowlist.hcl");

#[derive(Deserialize)]
struct AllowlistFile {
    allowlist: BTreeMap<String, AllowlistPolicy>,
}

/// One `allowlist "<pack>" {}` block. Every list is a set of bare hosts; the
/// split mirrors why each host is acceptable (internal, BAA-covered, or a
/// browser-loaded static asset that carries no PHI).
#[derive(Deserialize, Default)]
struct AllowlistPolicy {
    #[serde(default)]
    endpoints: Vec<String>,
    #[serde(default)]
    baa_endpoints: Vec<String>,
    #[serde(default)]
    asset_endpoints: Vec<String>,
}

/// Every host the pack's policies/network-allowlist.hcl declares, when the
/// pack ships one. The ai-allowlist gate's evidence pass (#3) checks host
/// literals in the scaffold source against exactly this list — so widening
/// an app's reach is a signed pack revision, never an app-level edit.
pub fn network_allowlist(pack_id: &str) -> Option<Vec<String>> {
    static POST_OP: LazyLock<Vec<String>> = LazyLock::new(|| {
        let file: AllowlistFile = hcl::from_str(POST_OP_MONITOR_ALLOWLIST)
            .expect("embedded network-allowlist.hcl must parse");
        file.allowlist
            .into_values()
            .flat_map(|p| {
                p.endpoints
                    .into_iter()
                    .chain(p.baa_endpoints)
                    .chain(p.asset_endpoints)
            })
            .collect()
    });
    match pack_id {
        "post-op-monitor" => Some(POST_OP.clone()),
        _ => None,
    }
}

fn decode_pack(source: &str) -> Result<PackManifest> {
    let file: PackFile = hcl::from_str(source).context("invalid pack.hcl")?;
    if file.pack.len() != 1 {
        bail!("pack.hcl must declare exactly one pack block");
    }
    let (id, mut manifest) = file.pack.into_iter().next().expect("length checked");
    let valid_id = (1..=96).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-');
    if !valid_id {
        bail!("pack id must be a lowercase ASCII slug");
    }
    manifest.id = id;
    Ok(manifest)
}

pub fn parse_pack(source: &str) -> Result<PackManifest> {
    let manifest = decode_pack(source)?;
    if !TRUSTED_SIGNERS.contains(&manifest.signed_by.as_str()) {
        bail!(
            "pack {} signed by untrusted key {:?} — refusing to register",
            manifest.id,
            manifest.signed_by
        );
    }
    Ok(manifest)
}

/// Parse a pack manifest that came back with an owned export. This checks
/// its shape and ownership label but never promotes it into the trusted
/// built-in signer chain. The caller must resolve `based_on` to a trusted
/// built-in pack and keep that pack's gates authoritative.
pub fn parse_owned_pack(source: &str) -> Result<PackManifest> {
    let manifest = decode_pack(source)?;
    if manifest.signed_by != "untrusted-practice-export" {
        bail!(
            "owned pack {} must be labeled untrusted-practice-export",
            manifest.id
        );
    }
    let based_on = manifest
        .based_on
        .as_deref()
        .context("owned pack is missing based_on")?;
    if based_on == manifest.id {
        bail!("owned pack cannot be based on itself");
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
        assert_eq!(packs.len(), 17);
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
                    assert!(
                        files.iter().any(|(p, _)| *p == "scaffold/Cargo.lock"),
                        "{}: a runnable scaffold has a reproducible dependency lock",
                        pack.id
                    );
                    assert_eq!(
                        pack.quality_contract.as_deref(),
                        Some("artifact-quality.json")
                    );
                    assert!(files.iter().any(|(p, _)| *p == "artifact-quality.json"));
                }
                (None, None) => {} // not yet converted (#5) — honestly absent
                (path, files) => panic!(
                    "{}: scaffold_path ({path:?}) and embedded sources ({}) disagree",
                    pack.id,
                    files.map(|f| f.len()).unwrap_or(0)
                ),
            }
        }
        assert!(builtin_packs()
            .iter()
            .all(|pack| pack.scaffold_path.as_deref() == Some("scaffold")));
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

    #[test]
    fn owned_pack_is_structural_metadata_not_a_registry_signature() {
        let owned = r#"
            pack "my-starter" {
              name = "My starter"
              description = "Owned source"
              profile = "web"
              tier = 1
              wave = 1
              signed_by = "untrusted-practice-export"
              based_on = "post-op-monitor"
              scaffold = ["one bounded feature"]
              prewired = ["synthetic-only"]
              gates = ["synthetic-only"]
              synthetic_dataset = "owned synthetic fixture"
            }
        "#;
        assert!(parse_pack(owned).is_err());
        let parsed = parse_owned_pack(owned).unwrap();
        assert_eq!(parsed.based_on.as_deref(), Some("post-op-monitor"));

        let forged = owned.replace("untrusted-practice-export", "platform-root-v1");
        assert!(parse_owned_pack(&forged).is_err());
        let duplicate = format!("{owned}\n{owned}");
        assert!(parse_owned_pack(&duplicate).is_err());
    }
}
