//! Adversarial evidence contract (#3): the issue's bar. A deliberately
//! broken scaffold — un-audited route, PHI fields bypassing encryption, a
//! rogue AI endpoint, no synthetic boot guard — must fail the corresponding
//! evidence gate, with the defect named. The fixture is embedded test data
//! only (tests/fixtures/): it is not a shipped pack, never parses through
//! the registry, and never lists — the pressure test asserts that too.

use std::collections::BTreeSet;

use rust_proof_service::gates::{self, Basis, EvidenceContext, GateReport, GateStatus};
use rust_proof_service::packs::{self, PackSourceFile};
use rust_proof_service::state::{AppRecord, DataSource, Stage};

const BROKEN_SCAFFOLD: &[PackSourceFile] = &[(
    "scaffold/src/main.rs",
    include_str!("fixtures/broken-scaffold-main.rs"),
)];

const EVIDENCE_GATES: [&str; 4] = [
    "audit-log",
    "phi-encryption",
    "ai-allowlist",
    "synthetic-only",
];

fn broken_ctx() -> EvidenceContext {
    EvidenceContext {
        files: BROKEN_SCAFFOLD,
        // Same signed allowlist the real pack ships — the fixture's rogue
        // host must fail against the genuine policy, not a strawman.
        allowlist: packs::network_allowlist("post-op-monitor").unwrap(),
    }
}

/// An app record that CLAIMS every control — the adversarial premise: a
/// buggy or malicious generator self-reports full compliance. Evidence must
/// not care.
fn lying_app() -> AppRecord {
    AppRecord {
        id: "app-broken".to_string(),
        name: "broken scaffold".to_string(),
        prompt: "adversarial fixture".to_string(),
        pack: "post-op-monitor".to_string(),
        stage: Stage::Sandbox,
        data_source: DataSource::Synthetic("post-op demo (12 pts)".to_string()),
        controls: EVIDENCE_GATES
            .iter()
            .map(|g| g.to_string())
            .collect::<BTreeSet<_>>(),
        external_calls: vec![],
        features: vec![],
        routes: 0,
        addenda: vec![],
        current_version: 1,
        reviewer_note: None,
        allocation: None,
        attestation: None,
        tenant: "meridian".to_string(),
    }
}

fn broken_report() -> GateReport {
    let required: Vec<String> = EVIDENCE_GATES.iter().map(|g| g.to_string()).collect();
    gates::preflight_with_context(&lying_app(), &required, Some(&broken_ctx()))
}

fn outcome(report: &GateReport, id: &str) -> GateStatus {
    report
        .results
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("{id} missing from report"))
        .outcome
        .clone()
}

fn fail_reason(report: &GateReport, id: &str) -> String {
    match outcome(report, id) {
        GateStatus::Fail { reason, .. } => reason,
        other => panic!("{id} must FAIL on the broken scaffold, got {other:?}"),
    }
}

#[test]
fn every_evidence_gate_fails_the_broken_scaffold_despite_claimed_controls() {
    let report = broken_report();
    assert_eq!(report.passed, 0);
    assert_eq!(report.stubbed, 0);
    assert!(!report.green);
    for result in &report.results {
        assert_eq!(result.basis, Basis::Evidence, "{}", result.id);
        assert!(
            matches!(result.outcome, GateStatus::Fail { .. }),
            "{} passed a scaffold built to fail it: {:?}",
            result.id,
            result.outcome
        );
    }
}

#[test]
fn audit_log_evidence_names_the_unaudited_route() {
    let reason = fail_reason(&broken_report(), "audit-log");
    assert!(reason.contains("/admin/export-everything"), "{reason}");
    assert!(
        !reason.contains("/visits"),
        "routes registered before the audit layer are not the defect: {reason}"
    );
}

#[test]
fn phi_encryption_evidence_names_both_bypass_shapes() {
    let reason = fail_reason(&broken_report(), "phi-encryption");
    // Shape 1: PHI fields with no declared encryption path at all.
    assert!(reason.contains("Visit"), "{reason}");
    assert!(reason.contains("patient_name"), "{reason}");
    assert!(reason.contains("no declared encryption path"), "{reason}");
    // Shape 2: a vault-transit claim with no call site to back it.
    assert!(reason.contains("Claim"), "{reason}");
    assert!(reason.contains("member_id"), "{reason}");
    assert!(reason.contains("no hipaa_core::encrypt_field"), "{reason}");
}

#[test]
fn ai_allowlist_evidence_names_the_rogue_host() {
    let reason = fail_reason(&broken_report(), "ai-allowlist");
    assert!(reason.contains("api.openai.com"), "{reason}");
    assert!(reason.contains("network-allowlist"), "{reason}");
}

#[test]
fn synthetic_only_evidence_requires_the_boot_guard_in_source() {
    let reason = fail_reason(&broken_report(), "synthetic-only");
    assert!(reason.contains("SYNTHETIC-notice boot guard"), "{reason}");
}

#[test]
fn positive_control_the_real_scaffold_passes_and_its_stub_stays_a_stub() {
    // The same gates, pointed at the genuine post-op scaffold: three pass
    // on evidence, and the encryption stub reports stubbed — never pass.
    let ctx = EvidenceContext::for_pack("post-op-monitor").expect("post-op ships a scaffold");
    let required: Vec<String> = EVIDENCE_GATES.iter().map(|g| g.to_string()).collect();
    let report = gates::preflight_with_context(&lying_app(), &required, Some(&ctx));
    assert_eq!(outcome(&report, "audit-log"), GateStatus::Pass);
    assert_eq!(outcome(&report, "ai-allowlist"), GateStatus::Pass);
    assert_eq!(outcome(&report, "synthetic-only"), GateStatus::Pass);
    assert!(matches!(
        outcome(&report, "phi-encryption"),
        GateStatus::Stubbed { .. }
    ));
    assert!(report.green, "labeled stubs satisfy; failures never do");
    assert_eq!(report.summary(), "3/4 (1 stubbed)");
}
