//! Contract tests for the control store (#7): the lifecycle transition set
//! is defined ONCE (src/state.rs) and every enforcer derives from it — the
//! in-memory `valid_transition` check in deploy.rs and the Postgres
//! `app_valid_state` seed in migrations/0001_init.sql. These tests pin the
//! two to each other and prove an illegal transition is structurally
//! impossible in memory (the DB trigger enforces the same table server-side;
//! the staging pressure test exercises that path).

use std::collections::BTreeSet;

use rust_proof_service::deploy;
use rust_proof_service::gates;
use rust_proof_service::state::{
    valid_transition, AppRecord, DataSource, Stage, OP_STATUSES, VALID_STAGE_TRANSITIONS,
};
use rust_proof_service::store::MIGRATION;

fn app_in(stage: Stage) -> AppRecord {
    AppRecord {
        id: "state-test".to_string(),
        name: "state test".to_string(),
        prompt: "a transition test app".to_string(),
        pack: "post-op-monitor".to_string(),
        stage,
        data_source: DataSource::Synthetic("synthea-postop-v1".to_string()),
        controls: BTreeSet::new(),
        external_calls: Vec::new(),
        features: vec!["symptom check-in".to_string()],
        routes: 1,
        addenda: Vec::new(),
        current_version: 1,
        reviewer_note: None,
        allocation: None,
        attestation: None,
        tenant: "meridian".to_string(),
    }
}

// ---------- one truth: the SQL seed IS the Rust const ----------

/// Extract the (prior, current) pairs seeded into app_valid_state by the
/// migration — the literal Boundary pattern (steering §5).
fn seeded_pairs() -> BTreeSet<(String, String)> {
    let seed = MIGRATION
        .split("INSERT INTO app_valid_state")
        .nth(1)
        .expect("migration seeds app_valid_state")
        .split("ON CONFLICT")
        .next()
        .expect("seed ends with ON CONFLICT DO NOTHING");
    // Rows look like ('sandbox', 'live'); collect quoted tokens pairwise
    // from the VALUES list (the column list contains no quotes).
    let quoted: Vec<String> = seed
        .split('\'')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect();
    assert!(
        quoted.len().is_multiple_of(2) && !quoted.is_empty(),
        "seed rows must be (prior, current) pairs: {quoted:?}"
    );
    quoted
        .chunks(2)
        .map(|c| (c[0].clone(), c[1].clone()))
        .collect()
}

#[test]
fn sql_transition_seed_matches_the_rust_const() {
    let from_const: BTreeSet<(String, String)> = VALID_STAGE_TRANSITIONS
        .iter()
        .map(|(p, n)| (p.as_str().to_string(), n.as_str().to_string()))
        .collect();
    assert_eq!(
        seeded_pairs(),
        from_const,
        "migrations/0001_init.sql app_valid_state seed drifted from \
         state::VALID_STAGE_TRANSITIONS — they must be the same set"
    );
    // And the const has no duplicate rows hiding a miscount.
    assert_eq!(from_const.len(), VALID_STAGE_TRANSITIONS.len());
}

#[test]
fn sql_stage_and_status_spellings_match_the_enums() {
    // apps.stage CHECK carries every Stage spelling…
    for stage in [Stage::Sandbox, Stage::Live] {
        assert!(
            MIGRATION.contains(&format!("'{}'", stage.as_str())),
            "migration must name stage {:?}",
            stage.as_str()
        );
    }
    // …and operations.status CHECK carries every OpStatus spelling.
    let check = MIGRATION
        .split("CHECK (status IN (")
        .nth(1)
        .expect("operations.status has a CHECK constraint")
        .split(')')
        .next()
        .unwrap();
    for status in OP_STATUSES {
        assert!(
            check.contains(&format!("'{}'", status.as_str())),
            "operations.status CHECK must allow {:?}",
            status.as_str()
        );
    }
}

// ---------- the in-memory enforcement consults the same table ----------

#[test]
fn transition_table_allows_exactly_promote_and_rollback() {
    assert!(valid_transition(Stage::Sandbox, Stage::Live));
    assert!(valid_transition(Stage::Live, Stage::Sandbox));
    assert!(!valid_transition(Stage::Sandbox, Stage::Sandbox));
    assert!(!valid_transition(Stage::Live, Stage::Live));
}

/// Structurally impossible: the only mutators of `stage` are
/// deploy::promote and deploy::rollback, and both refuse any pair not in
/// VALID_STAGE_TRANSITIONS — there is no code path that can set an illegal
/// stage. (The DB trigger re-enforces the same table for defense in depth.)
#[test]
fn illegal_transitions_are_refused_and_leave_the_record_untouched() {
    // live→live via promote: refused before any mutation.
    let mut live = app_in(Stage::Live);
    let report = gates::preflight(&live, &[]);
    let before = live.clone();
    let err = deploy::promote(&mut live, &report, "Dr. A. Osei", "a-1".to_string())
        .expect_err("promoting a live app must fail");
    assert!(err.to_string().contains("already live"), "{err}");
    assert_eq!(live.stage, before.stage);
    assert_eq!(live.current_version, before.current_version);

    // sandbox→sandbox via rollback: refused before any mutation.
    let mut sandboxed = app_in(Stage::Sandbox);
    let err = deploy::rollback(&mut sandboxed, "synthea-postop-v1")
        .expect_err("rolling back a sandbox app must fail");
    assert!(err.to_string().contains("no live allocation"), "{err}");
    assert_eq!(sandboxed.stage, Stage::Sandbox);
}

/// A legal transition still passes through the same table — promote flips
/// sandbox→live exactly as VALID_STAGE_TRANSITIONS says it may.
#[test]
fn legal_transition_passes_the_same_table() {
    let mut app = app_in(Stage::Sandbox);
    app.controls.insert("auto-logoff".to_string());
    let required = vec!["auto-logoff".to_string()];
    let report = gates::preflight(&app, &required);
    assert!(report.green, "one wired control-basis gate is green");
    deploy::promote(&mut app, &report, "Dr. A. Osei", "a-2".to_string())
        .expect("sandbox→live is legal");
    assert_eq!(app.stage, Stage::Live);
    deploy::rollback(&mut app, "synthea-postop-v1").expect("live→sandbox is legal");
    assert_eq!(app.stage, Stage::Sandbox);
}
