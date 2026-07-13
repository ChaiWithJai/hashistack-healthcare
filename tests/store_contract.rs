//! Contract tests for the control store (#7): the lifecycle transition set
//! is defined ONCE (src/state.rs) and every enforcer derives from it — the
//! in-memory `valid_transition` check in deploy.rs and the Postgres
//! `app_valid_state` seed in migrations/0001_init.sql. These tests pin the
//! two to each other and prove an illegal transition is structurally
//! impossible in memory (the DB trigger enforces the same table server-side;
//! the staging pressure test exercises that path).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rust_proof_service::deploy;
use rust_proof_service::gates;
use rust_proof_service::identity::{Principal, Registry};
use rust_proof_service::packs;
use rust_proof_service::state::{
    valid_transition, Allocation, AppRecord, DataSource, Platform, SharedPlatform, Stage,
    OP_STATUSES, VALID_STAGE_TRANSITIONS,
};
use rust_proof_service::store::{self, PgStore, MIGRATION};
use rust_proof_service::workspace::{
    source_digest, CandidateFile, CandidatePatch, CheckStatus, Treatment, TreatmentPlan,
    VerificationCheck, VerificationReport, WorkspaceRecord, EXECUTABLE_CHECK_IDS,
};

/// The dev registry's meridian clinician — the co-signing principal (#10).
fn dr_osei() -> Principal {
    Registry::dev_default()
        .by_token("dev-token-osei")
        .unwrap()
        .clone()
}

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

#[test]
fn migration_owns_the_editable_source_workspace() {
    assert!(MIGRATION.contains("CREATE TABLE IF NOT EXISTS source_workspaces"));
    assert!(MIGRATION.contains("REFERENCES apps(app_id) ON DELETE CASCADE"));
    assert!(MIGRATION.contains("record     JSONB NOT NULL"));
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
    let err = deploy::promote(
        &mut live,
        &report,
        &dr_osei(),
        None,
        "a-1".to_string(),
        false,
    )
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
    deploy::promote(
        &mut app,
        &report,
        &dr_osei(),
        None,
        "a-2".to_string(),
        false,
    )
    .expect("sandbox→live is legal");
    assert_eq!(app.stage, Stage::Live);
    deploy::rollback(&mut app, "synthea-postop-v1").expect("live→sandbox is legal");
    assert_eq!(app.stage, Stage::Sandbox);
}

// ---------- rollback saga state survives a real PgStore restart ----------

fn postgres_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn test_control_db_url() -> String {
    std::env::var("TEST_CONTROL_DB_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .expect("TEST_CONTROL_DB_URL is required for the ignored real-PgStore restart contracts")
}

fn rollback_app(id: String, workload_stopped: bool) -> AppRecord {
    let mut app = app_in(Stage::Live);
    app.id = id;
    app.allocation = Some(Allocation {
        id: "alloc-restart-proof".to_string(),
        pool: "prod".to_string(),
        region: "nyc3".to_string(),
        image: "client@sha256:restart-proof".to_string(),
        profile: "small".to_string(),
        database: "vault dynamic postgres".to_string(),
        credentials: "vault lease handle".to_string(),
        app_version: 1,
        url: "https://restart-proof.invalid".to_string(),
        healthy: false,
        cleanup_pending: true,
        cleanup_workload_stopped: workload_stopped,
        cleanup_error: None,
        deployed_at: 1,
        nomad_eval_id: Some("eval-restart-proof".to_string()),
        vault_transit_key: Some("tenant-meridian".to_string()),
        vault_lease_id: Some("database/creds/tenant-app/restart-proof".to_string()),
        vault_db_username: Some("v-restart-proof".to_string()),
        vault_lease_ttl_secs: Some(3600),
    });
    app
}

async fn assert_rollback_state_survives_pg_restart(workload_stopped: bool) {
    let url = test_control_db_url();
    let _serial = postgres_test_lock().lock().await;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let id = format!(
        "rollback-restart-{}-{}-{nonce}",
        if workload_stopped {
            "verified"
        } else {
            "requested"
        },
        std::process::id()
    );
    let expected = rollback_app(id.clone(), workload_stopped);

    // First process: attach the real store and persist the intermediate
    // ownership record exactly as the rollback handler does before it moves
    // on to the next saga step.
    let first_store = Arc::new(PgStore::connect(&url).await.expect("connect first PgStore"));
    let mut first_state = Platform::new(Vec::new());
    first_state.apps.insert(id.clone(), expected.clone());
    first_state.store = Some(first_store);
    let first: SharedPlatform = Arc::new(RwLock::new(first_state));
    store::write_through(&first, &[&id], None)
        .await
        .expect("persist rollback intermediate state");
    drop(first);

    // Restart: a new connection and a fresh in-memory Platform must recover
    // the state from JSONB, including the separately durable stop bit and all
    // handles needed to continue cleanup.
    let second_store = PgStore::connect(&url)
        .await
        .expect("connect restarted PgStore");
    let mut restarted = Platform::new(packs::builtin_packs());
    second_store
        .load(&mut restarted)
        .await
        .expect("load restarted platform");
    let recovered = restarted.apps.get(&id).expect("rollback app recovered");
    assert_eq!(recovered.stage, Stage::Live);
    let allocation = recovered
        .allocation
        .as_ref()
        .expect("cleanup allocation retained");
    assert!(allocation.cleanup_pending);
    assert_eq!(allocation.cleanup_workload_stopped, workload_stopped);
    assert!(!allocation.healthy);
    assert_eq!(
        allocation.nomad_eval_id,
        expected.allocation.as_ref().unwrap().nomad_eval_id
    );
    assert_eq!(
        allocation.vault_lease_id,
        expected.allocation.as_ref().unwrap().vault_lease_id
    );
    assert_eq!(
        allocation.vault_db_username,
        expected.allocation.as_ref().unwrap().vault_db_username
    );
}

#[tokio::test]
#[ignore = "requires TEST_CONTROL_DB_URL pointing to disposable Postgres"]
async fn postgres_restart_recovers_rollback_requested_before_workload_stop() {
    assert_rollback_state_survives_pg_restart(false).await;
}

#[tokio::test]
#[ignore = "requires TEST_CONTROL_DB_URL pointing to disposable Postgres"]
async fn postgres_restart_recovers_rollback_verified_cleanup_before_sandbox_transition() {
    assert_rollback_state_survives_pg_restart(true).await;
}

#[tokio::test]
#[ignore = "requires TEST_CONTROL_DB_URL pointing to disposable Postgres"]
async fn postgres_restart_recovers_the_editable_source_workspace() {
    let url = test_control_db_url();
    let _serial = postgres_test_lock().lock().await;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let id = format!("workspace-restart-{}-{nonce}", std::process::id());
    let app = {
        let mut app = app_in(Stage::Sandbox);
        app.id = id.clone();
        app
    };
    let mut workspace = WorkspaceRecord::new(
        id.clone(),
        BTreeMap::from([
            ("README.md".to_string(), "# Durable workspace\n".to_string()),
            (
                "web/src/App.svelte".to_string(),
                "<h1>Ready</h1>\n".to_string(),
            ),
        ]),
        1,
    );
    workspace
        .set_plan(
            TreatmentPlan {
                problem: "Make follow-up safer".to_string(),
                recommended_treatment_id: "guided".to_string(),
                treatments: vec![
                    Treatment {
                        id: "calm".to_string(),
                        label: "Calm follow-up".to_string(),
                        user_outcome: "Keep the next action visible.".to_string(),
                        screen_changes: vec!["Add one focused step.".to_string()],
                        data_changes: vec![],
                        safety_notes: vec!["Synthetic data only.".to_string()],
                    },
                    Treatment {
                        id: "guided".to_string(),
                        label: "Guided follow-up".to_string(),
                        user_outcome: "Explain why the next action matters.".to_string(),
                        screen_changes: vec!["Use a guided panel.".to_string()],
                        data_changes: vec![],
                        safety_notes: vec!["Synthetic data only.".to_string()],
                    },
                    Treatment {
                        id: "compact".to_string(),
                        label: "Compact follow-up".to_string(),
                        user_outcome: "Fit the action into the existing card.".to_string(),
                        screen_changes: vec!["Use the existing card.".to_string()],
                        data_changes: vec![],
                        safety_notes: vec!["Synthetic data only.".to_string()],
                    },
                ],
                acceptance_checks: vec!["The next action is visible.".to_string()],
            },
            2,
        )
        .expect("valid treatment plan");
    workspace.select("guided", 3).expect("select treatment");
    let candidate_files = BTreeMap::from([
        ("README.md".to_string(), "# Durable workspace\n".to_string()),
        (
            "web/src/App.svelte".to_string(),
            "<h1>Durably reviewed</h1>\n".to_string(),
        ),
    ]);
    workspace
        .review_candidate(
            "candidate-restart-proof".to_string(),
            CandidatePatch {
                summary: "Make the accepted source survive restart.".to_string(),
                files: vec![CandidateFile {
                    path: "web/src/App.svelte".to_string(),
                    content: "<h1>Durably reviewed</h1>\n".to_string(),
                    reason: "The clinician selected the guided treatment.".to_string(),
                }],
                verification_commands: vec![],
            },
            VerificationReport {
                id: "verify-v1-restart-proof".to_string(),
                workspace_digest: source_digest(&candidate_files),
                profile_digest: "sha256:restart-profile".to_string(),
                checks: EXECUTABLE_CHECK_IDS
                    .iter()
                    .map(|id| VerificationCheck {
                        id: (*id).to_string(),
                        status: CheckStatus::Pass,
                        detail: "passed in the bounded verifier".to_string(),
                    })
                    .collect(),
                passed: true,
                verified_at: 4,
            },
            4,
        )
        .expect("review verified candidate");

    let first_store = Arc::new(PgStore::connect(&url).await.expect("connect first PgStore"));
    let mut first_state = Platform::new(Vec::new());
    first_state.apps.insert(id.clone(), app);
    first_state.workspaces.insert(id.clone(), workspace.clone());
    first_state.store = Some(first_store);
    let first: SharedPlatform = Arc::new(RwLock::new(first_state));
    store::write_through(&first, &[&id], None)
        .await
        .expect("persist editable workspace");
    drop(first);

    let second_store = Arc::new(
        PgStore::connect(&url)
            .await
            .expect("connect restarted PgStore"),
    );
    let mut restarted = Platform::new(packs::builtin_packs());
    let counts = second_store
        .load(&mut restarted)
        .await
        .expect("load restarted platform");
    assert!(counts.1 >= 1, "load reports durable workspaces");
    assert_eq!(restarted.workspaces.get(&id), Some(&workspace));

    // Continue after restart: accepting the recovered candidate must produce
    // another durable checkpoint that a third process loads byte-for-byte.
    restarted
        .workspaces
        .get_mut(&id)
        .expect("recovered workspace")
        .accept("candidate-restart-proof", 5)
        .expect("accept recovered candidate");
    let accepted = restarted.workspaces[&id].clone();
    restarted.store = Some(second_store);
    let second: SharedPlatform = Arc::new(RwLock::new(restarted));
    store::write_through(&second, &[&id], None)
        .await
        .expect("persist accepted checkpoint");
    drop(second);

    let third_store = PgStore::connect(&url).await.expect("connect third PgStore");
    let mut third = Platform::new(packs::builtin_packs());
    third_store
        .load(&mut third)
        .await
        .expect("load accepted checkpoint");
    assert_eq!(third.workspaces.get(&id), Some(&accepted));
    assert_eq!(accepted.accepted.version, 1);
    assert_eq!(
        accepted.accepted.files["web/src/App.svelte"],
        "<h1>Durably reviewed</h1>\n"
    );
}
