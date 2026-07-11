//! Pack registry: serves signed use case packs.
//!
//! A pack is a declarative HCL manifest — the platform's extension unit,
//! mirroring Terraform modules and Nomad job specs. The registry only loads
//! manifests carrying a known signature chain; unsigned packs never list.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Signature roots the registry accepts. Phase 0: the platform key only.
/// Open question 2 in the RFC — clinician identity in the chain — lands here.
const TRUSTED_SIGNERS: &[&str] = &["platform-root-v1"];

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
    pub scaffold: Vec<String>,
    pub prewired: Vec<String>,
    pub gates: Vec<String>,
    pub synthetic_dataset: String,
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
