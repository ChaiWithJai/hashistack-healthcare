//! Source workspace state owned by the Rust control plane.
//!
//! A hosted agent may propose treatments and files. It cannot change the
//! accepted checkpoint. Rust validates the proposal, records verification,
//! shows a diff, and advances the checkpoint only after explicit acceptance.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SOURCE_FILES: usize = 64;
pub const MAX_SOURCE_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Treatment {
    pub id: String,
    pub label: String,
    pub user_outcome: String,
    pub screen_changes: Vec<String>,
    #[serde(default)]
    pub data_changes: Vec<String>,
    #[serde(default)]
    pub safety_notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreatmentPlan {
    pub problem: String,
    pub recommended_treatment_id: String,
    pub treatments: Vec<Treatment>,
    pub acceptance_checks: Vec<String>,
}

impl TreatmentPlan {
    pub fn validate(&self) -> Result<(), String> {
        if !(2..=3).contains(&self.treatments.len()) {
            return Err("a plan must contain two or three treatments".into());
        }
        let ids = self
            .treatments
            .iter()
            .map(|item| item.id.as_str())
            .collect::<BTreeSet<_>>();
        if ids.len() != self.treatments.len() {
            return Err("treatment ids must be unique".into());
        }
        if !ids.contains(self.recommended_treatment_id.as_str()) {
            return Err("recommended treatment must exist".into());
        }
        if self.problem.trim().is_empty() || self.acceptance_checks.is_empty() {
            return Err("problem and acceptance checks are required".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateFile {
    pub path: String,
    pub content: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatePatch {
    pub summary: String,
    pub files: Vec<CandidateFile>,
    #[serde(default)]
    pub verification_commands: Vec<String>,
}

impl CandidatePatch {
    pub fn validate(&self) -> Result<(), String> {
        if self.files.is_empty() || self.files.len() > MAX_SOURCE_FILES {
            return Err(format!(
                "candidate must contain 1 to {MAX_SOURCE_FILES} files"
            ));
        }
        let mut paths = BTreeSet::new();
        let mut bytes = 0usize;
        for file in &self.files {
            validate_path(&file.path)?;
            if !paths.insert(file.path.as_str()) {
                return Err(format!("duplicate candidate path: {}", file.path));
            }
            bytes = bytes
                .checked_add(file.content.len())
                .ok_or_else(|| "candidate size overflow".to_string())?;
            if file.reason.trim().is_empty() {
                return Err(format!("candidate reason is missing: {}", file.path));
            }
        }
        if bytes > MAX_SOURCE_BYTES {
            return Err(format!("candidate exceeds {MAX_SOURCE_BYTES} bytes"));
        }
        Ok(())
    }
}

fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe candidate path: {path}"));
    }
    if !["web/", "server/", "tests/", "synthetic/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Err(format!("candidate path is outside the workspace: {path}"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePhase {
    Described,
    TreatmentsReady,
    TreatmentSelected,
    Generating,
    CandidateReady,
    Verifying,
    ReviewRequired,
    Accepted,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Fail,
}

pub const EXECUTABLE_CHECK_IDS: [&str; 5] = [
    "workspace.structure.v1",
    "web.svelte-check.v1",
    "web.svelte-build.v1",
    "server.cargo-test.v1",
    "browser.synthetic-smoke.v1",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationCheck {
    pub id: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReport {
    pub id: String,
    pub workspace_digest: String,
    pub profile_digest: String,
    pub checks: Vec<VerificationCheck>,
    pub passed: bool,
    pub verified_at: u64,
}

impl VerificationReport {
    pub fn validate(&self) -> Result<(), String> {
        if !self.id.starts_with("verify-v1-")
            || !self.workspace_digest.starts_with("sha256:")
            || !self.profile_digest.starts_with("sha256:")
        {
            return Err("verification evidence identity is invalid".into());
        }
        if self.checks.len() != EXECUTABLE_CHECK_IDS.len()
            || self
                .checks
                .iter()
                .zip(EXECUTABLE_CHECK_IDS)
                .any(|(check, id)| check.id != id)
        {
            return Err("verification checks or order do not match executable profile v1".into());
        }
        let all_pass = !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|check| check.status == CheckStatus::Pass);
        if self.passed != all_pass {
            return Err("verification summary does not match its checks".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub before_sha256: Option<String>,
    pub after_sha256: String,
    pub unified: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    pub digest: String,
    pub files: BTreeMap<String, String>,
    pub accepted_at: u64,
}

impl Checkpoint {
    pub fn new(version: u32, files: BTreeMap<String, String>, accepted_at: u64) -> Self {
        let digest = source_digest(&files);
        Self {
            version,
            digest,
            files,
            accepted_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub base_version: u32,
    pub treatment_id: String,
    pub summary: String,
    pub files: Vec<CandidateFile>,
    pub diff: Vec<FileDiff>,
    pub verification: VerificationReport,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub app_id: String,
    pub phase: WorkspacePhase,
    pub treatment_plan: Option<TreatmentPlan>,
    pub selected_treatment_id: Option<String>,
    pub accepted: Checkpoint,
    pub candidate: Option<Candidate>,
    pub failure: Option<String>,
    pub updated_at: u64,
}

impl WorkspaceRecord {
    pub fn new(app_id: String, files: BTreeMap<String, String>, now: u64) -> Self {
        Self {
            app_id,
            phase: WorkspacePhase::Described,
            treatment_plan: None,
            selected_treatment_id: None,
            accepted: Checkpoint::new(0, files, now),
            candidate: None,
            failure: None,
            updated_at: now,
        }
    }

    pub fn set_plan(&mut self, plan: TreatmentPlan, now: u64) -> Result<(), String> {
        plan.validate()?;
        self.treatment_plan = Some(plan);
        self.selected_treatment_id = None;
        self.candidate = None;
        self.phase = WorkspacePhase::TreatmentsReady;
        self.updated_at = now;
        Ok(())
    }

    pub fn select(&mut self, treatment_id: &str, now: u64) -> Result<(), String> {
        let plan = self
            .treatment_plan
            .as_ref()
            .ok_or_else(|| "treatment plan is missing".to_string())?;
        if !plan.treatments.iter().any(|item| item.id == treatment_id) {
            return Err("selected treatment is not in this plan".into());
        }
        self.selected_treatment_id = Some(treatment_id.to_string());
        self.phase = WorkspacePhase::TreatmentSelected;
        self.updated_at = now;
        Ok(())
    }

    pub fn review_candidate(
        &mut self,
        id: String,
        patch: CandidatePatch,
        report: VerificationReport,
        now: u64,
    ) -> Result<(), String> {
        patch.validate()?;
        report.validate()?;
        let treatment_id = self
            .selected_treatment_id
            .clone()
            .ok_or_else(|| "select a treatment before generation".to_string())?;
        let diff = diff_files(&self.accepted.files, &patch.files);
        let mut verified_files = self.accepted.files.clone();
        for file in &patch.files {
            verified_files.insert(file.path.clone(), file.content.clone());
        }
        if report.workspace_digest != source_digest(&verified_files) {
            return Err("verification report does not match candidate bytes".into());
        }
        self.candidate = Some(Candidate {
            id,
            base_version: self.accepted.version,
            treatment_id,
            summary: patch.summary,
            files: patch.files,
            diff,
            verification: report.clone(),
            created_at: now,
        });
        self.phase = if report.passed {
            WorkspacePhase::ReviewRequired
        } else {
            WorkspacePhase::Failed
        };
        self.updated_at = now;
        Ok(())
    }

    pub fn accept(&mut self, candidate_id: &str, now: u64) -> Result<&Checkpoint, String> {
        if self.phase != WorkspacePhase::ReviewRequired {
            return Err("candidate is not ready for review".into());
        }
        let candidate = self
            .candidate
            .as_ref()
            .ok_or_else(|| "candidate is missing".to_string())?;
        if candidate.id != candidate_id || candidate.base_version != self.accepted.version {
            return Err("candidate is missing or stale".into());
        }
        if !candidate.verification.passed {
            return Err("failed verification cannot be accepted".into());
        }
        let mut files = self.accepted.files.clone();
        for file in &candidate.files {
            files.insert(file.path.clone(), file.content.clone());
        }
        self.accepted = Checkpoint::new(self.accepted.version + 1, files, now);
        self.candidate = None;
        self.phase = WorkspacePhase::Accepted;
        self.updated_at = now;
        Ok(&self.accepted)
    }

    pub fn reject(&mut self, candidate_id: &str, now: u64) -> Result<(), String> {
        if self.candidate.as_ref().map(|item| item.id.as_str()) != Some(candidate_id) {
            return Err("candidate is missing or stale".into());
        }
        self.candidate = None;
        self.phase = WorkspacePhase::Accepted;
        self.updated_at = now;
        Ok(())
    }
}

pub fn source_digest(files: &BTreeMap<String, String>) -> String {
    let mut hash = Sha256::new();
    for (path, content) in files {
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path.as_bytes());
        hash.update((content.len() as u64).to_be_bytes());
        hash.update(content.as_bytes());
    }
    format!("sha256:{:x}", hash.finalize())
}

fn content_hash(content: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(content.as_bytes()))
}

fn diff_files(before: &BTreeMap<String, String>, changes: &[CandidateFile]) -> Vec<FileDiff> {
    changes
        .iter()
        .map(|file| {
            let old = before.get(&file.path);
            FileDiff {
                path: file.path.clone(),
                before_sha256: old.map(|content| content_hash(content)),
                after_sha256: content_hash(&file.content),
                unified: format!(
                    "--- a/{path}\n+++ b/{path}\n@@ reviewed file @@\n-{old}\n+{new}\n",
                    path = file.path,
                    old = old.map(String::as_str).unwrap_or("<new file>"),
                    new = file.content
                ),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> TreatmentPlan {
        TreatmentPlan {
            problem: "reduce follow-up work".into(),
            recommended_treatment_id: "calm-list".into(),
            treatments: vec![
                Treatment {
                    id: "calm-list".into(),
                    label: "Calm list".into(),
                    user_outcome: "See what needs attention".into(),
                    screen_changes: vec!["queue".into()],
                    data_changes: vec![],
                    safety_notes: vec!["synthetic".into()],
                },
                Treatment {
                    id: "daily-view".into(),
                    label: "Daily view".into(),
                    user_outcome: "Work one day at a time".into(),
                    screen_changes: vec!["calendar".into()],
                    data_changes: vec![],
                    safety_notes: vec!["synthetic".into()],
                },
            ],
            acceptance_checks: vec!["the queue is visible".into()],
        }
    }

    fn patch(path: &str, content: &str) -> CandidatePatch {
        CandidatePatch {
            summary: "add view".into(),
            files: vec![CandidateFile {
                path: path.into(),
                content: content.into(),
                reason: "user selected it".into(),
            }],
            verification_commands: vec!["ignored by Rust".into()],
        }
    }

    fn pass(now: u64) -> VerificationReport {
        let files = BTreeMap::from([("web/x".to_string(), "good".to_string())]);
        VerificationReport {
            id: "verify-v1-test".into(),
            workspace_digest: source_digest(&files),
            profile_digest: "sha256:test".into(),
            checks: EXECUTABLE_CHECK_IDS
                .iter()
                .map(|id| VerificationCheck {
                    id: (*id).into(),
                    status: CheckStatus::Pass,
                    detail: "ok".into(),
                })
                .collect(),
            passed: true,
            verified_at: now,
        }
    }

    #[test]
    fn candidate_paths_and_budgets_are_enforced() {
        for path in [
            "../../.env",
            "/tmp/x",
            ".github/workflows/x",
            "web\\x",
            "web//x",
        ] {
            assert!(patch(path, "x").validate().is_err(), "{path}");
        }
        assert!(patch("web/src/routes/+page.svelte", "$state()")
            .validate()
            .is_ok());
        assert!(patch("web/large", &"x".repeat(MAX_SOURCE_BYTES + 1))
            .validate()
            .is_err());
    }

    #[test]
    fn plan_requires_unique_ids_and_a_real_recommendation() {
        let mut value = plan();
        value.treatments[1].id = value.treatments[0].id.clone();
        assert!(value.validate().is_err());
        let mut value = plan();
        value.recommended_treatment_id = "missing".into();
        assert!(value.validate().is_err());
    }

    #[test]
    fn failed_or_rejected_candidate_never_changes_the_checkpoint() {
        let mut workspace = WorkspaceRecord::new("app".into(), BTreeMap::new(), 1);
        workspace.set_plan(plan(), 2).unwrap();
        workspace.select("calm-list", 3).unwrap();
        let before = workspace.accepted.digest.clone();
        let failed = VerificationReport {
            id: "verify-v1-failed".into(),
            workspace_digest: source_digest(&BTreeMap::from([(
                "web/x".to_string(),
                "bad".to_string(),
            )])),
            profile_digest: "sha256:test".into(),
            checks: EXECUTABLE_CHECK_IDS
                .iter()
                .map(|id| VerificationCheck {
                    id: (*id).into(),
                    status: CheckStatus::Fail,
                    detail: "failed".into(),
                })
                .collect(),
            passed: false,
            verified_at: 4,
        };
        workspace
            .review_candidate("bad".into(), patch("web/x", "bad"), failed, 4)
            .unwrap();
        assert!(workspace.accept("bad", 5).is_err());
        assert_eq!(workspace.accepted.digest, before);
        workspace.reject("bad", 6).unwrap();
        assert_eq!(workspace.accepted.digest, before);
    }

    #[test]
    fn verified_candidate_advances_one_immutable_checkpoint() {
        let mut workspace = WorkspaceRecord::new("app".into(), BTreeMap::new(), 1);
        workspace.set_plan(plan(), 2).unwrap();
        workspace.select("calm-list", 3).unwrap();
        workspace
            .review_candidate("good".into(), patch("web/x", "good"), pass(4), 4)
            .unwrap();
        let accepted = workspace.accept("good", 5).unwrap();
        assert_eq!(accepted.version, 1);
        assert_eq!(accepted.files["web/x"], "good");
        assert!(workspace.accept("good", 6).is_err());
    }

    #[test]
    fn digest_is_stable_for_sorted_files_and_changes_with_bytes() {
        let a = BTreeMap::from([("web/b".into(), "2".into()), ("web/a".into(), "1".into())]);
        let b = BTreeMap::from([("web/a".into(), "1".into()), ("web/b".into(), "2".into())]);
        assert_eq!(source_digest(&a), source_digest(&b));
        let mut changed = b;
        changed.insert("web/b".into(), "3".into());
        assert_ne!(source_digest(&a), source_digest(&changed));
    }
}
