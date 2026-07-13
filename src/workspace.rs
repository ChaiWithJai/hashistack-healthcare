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
pub const MAX_TREATMENT_PLAN_BYTES: usize = 16 * 1024;
pub const MAX_REFINEMENT_BYTES: usize = 500;

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

impl Treatment {
    fn validate(&self) -> Result<(), String> {
        if !is_treatment_id(&self.id) {
            return Err(
                "treatment ids must be 1 to 64 lowercase ASCII letters, digits, or hyphens".into(),
            );
        }
        validate_bounded_text("treatment label", &self.label, 100)?;
        validate_bounded_text("treatment outcome", &self.user_outcome, 500)?;
        validate_text_list("screen changes", &self.screen_changes, 1, 6, 300)?;
        validate_text_list("data changes", &self.data_changes, 0, 6, 300)?;
        validate_text_list("safety notes", &self.safety_notes, 0, 6, 300)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresentationMode {
    #[default]
    TaskFirst,
    ContextFirst,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClinicianRefinement {
    #[serde(default)]
    pub presentation: PresentationMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emphasis: Option<String>,
}

impl ClinicianRefinement {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(emphasis) = &self.emphasis {
            validate_bounded_text("clinician emphasis", emphasis, MAX_REFINEMENT_BYTES)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreatmentSelection {
    pub treatment: Treatment,
    pub refinement: ClinicianRefinement,
    pub plan_digest: String,
    pub planner: AgentProvenance,
    pub selected_by: String,
    pub selected_at: u64,
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
        validate_bounded_text("treatment problem", &self.problem, 500)?;
        validate_text_list("acceptance checks", &self.acceptance_checks, 1, 8, 300)?;
        for treatment in &self.treatments {
            treatment.validate()?;
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
        if serde_json::to_vec(self)
            .map_err(|_| "treatment plan could not be serialized")?
            .len()
            > MAX_TREATMENT_PLAN_BYTES
        {
            return Err(format!(
                "treatment plan exceeds {MAX_TREATMENT_PLAN_BYTES} bytes"
            ));
        }
        Ok(())
    }
}

pub fn treatment_plan_digest(plan: &TreatmentPlan) -> String {
    let bytes = serde_json::to_vec(plan).expect("TreatmentPlan serialization cannot fail");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn validate_bounded_text(name: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max_bytes {
        return Err(format!("{name} must be 1 to {max_bytes} bytes"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{name} contains a control character"));
    }
    Ok(())
}

fn validate_text_list(
    name: &str,
    values: &[String],
    min_items: usize,
    max_items: usize,
    max_item_bytes: usize,
) -> Result<(), String> {
    if !(min_items..=max_items).contains(&values.len()) {
        return Err(format!(
            "{name} must contain {min_items} to {max_items} items"
        ));
    }
    for value in values {
        validate_bounded_text(name, value, max_item_bytes)?;
    }
    Ok(())
}

fn is_treatment_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes
            .first()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
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
    validate_safe_path(path)?;
    if !["web/", "server/", "tests/", "synthetic/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Err(format!("candidate path is outside the workspace: {path}"));
    }
    Ok(())
}

fn validate_safe_path(path: &str) -> Result<(), String> {
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
    Ok(())
}

fn validate_checkpoint_files(files: &BTreeMap<String, String>) -> Result<(), String> {
    if files.len() > MAX_SOURCE_FILES {
        return Err(format!(
            "accepted workspace exceeds {MAX_SOURCE_FILES} files"
        ));
    }
    let bytes = files
        .values()
        .try_fold(0usize, |total, content| total.checked_add(content.len()));
    if bytes.is_none_or(|bytes| bytes > MAX_SOURCE_BYTES) {
        return Err(format!(
            "accepted workspace exceeds {MAX_SOURCE_BYTES} bytes"
        ));
    }
    for path in files.keys() {
        validate_safe_path(path)?;
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

/// Durable, user-visible evidence of which bounded generation tier produced
/// the last plan or candidate. This records identifiers only; credentials,
/// prompts, and source bytes never enter provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProvenance {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

impl AgentProvenance {
    pub fn validate(&self) -> Result<(), String> {
        if self.provider.trim().is_empty() || self.provider.len() > 40 {
            return Err("agent provenance provider is invalid".into());
        }
        if self.model.trim().is_empty() || self.model.len() > 100 {
            return Err("agent provenance model is invalid".into());
        }
        if self
            .deployment_version
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 128)
        {
            return Err("agent provenance deployment version is invalid".into());
        }
        if self
            .fallback_reason
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 80)
        {
            return Err("agent provenance fallback reason is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub app_id: String,
    pub phase: WorkspacePhase,
    pub treatment_plan: Option<TreatmentPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_agent: Option<AgentProvenance>,
    pub selected_treatment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_treatment: Option<TreatmentSelection>,
    pub accepted: Checkpoint,
    pub candidate: Option<Candidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_agent: Option<AgentProvenance>,
    pub failure: Option<String>,
    pub updated_at: u64,
}

impl WorkspaceRecord {
    pub fn new(app_id: String, files: BTreeMap<String, String>, now: u64) -> Self {
        Self {
            app_id,
            phase: WorkspacePhase::Described,
            treatment_plan: None,
            plan_agent: None,
            selected_treatment_id: None,
            selected_treatment: None,
            accepted: Checkpoint::new(0, files, now),
            candidate: None,
            generation_agent: None,
            failure: None,
            updated_at: now,
        }
    }

    /// Re-establish every trust-boundary invariant before a JSONB record is
    /// admitted after restart. Durable bytes are not trusted merely because
    /// they came from our database: a stale migration, operator edit, or bug
    /// must fail boot rather than silently changing accepted source.
    pub fn validate_restored(&self) -> Result<(), String> {
        if self.app_id.trim().is_empty() {
            return Err("workspace app id is missing".into());
        }
        if self.accepted.digest != source_digest(&self.accepted.files) {
            return Err("accepted checkpoint digest does not match its files".into());
        }
        validate_checkpoint_files(&self.accepted.files)?;
        if let Some(plan) = &self.treatment_plan {
            plan.validate()?;
        }
        if let Some(provenance) = &self.plan_agent {
            provenance.validate()?;
            if self.treatment_plan.is_none() {
                return Err("plan agent provenance has no treatment plan".into());
            }
        }
        if let Some(provenance) = &self.generation_agent {
            provenance.validate()?;
        }
        if let Some(selected) = &self.selected_treatment_id {
            let plan = self
                .treatment_plan
                .as_ref()
                .ok_or_else(|| "selected treatment has no plan".to_string())?;
            if !plan.treatments.iter().any(|item| item.id == *selected) {
                return Err("selected treatment is not in the restored plan".into());
            }
        }
        if let Some(selection) = &self.selected_treatment {
            let plan = self
                .treatment_plan
                .as_ref()
                .ok_or_else(|| "selected treatment snapshot has no plan".to_string())?;
            selection.treatment.validate()?;
            selection.refinement.validate()?;
            selection.planner.validate()?;
            validate_bounded_text("selection actor", &selection.selected_by, 128)?;
            if self.selected_treatment_id.as_deref() != Some(selection.treatment.id.as_str()) {
                return Err("selected treatment snapshot does not match its id".into());
            }
            if selection.plan_digest != treatment_plan_digest(plan) {
                return Err("selected treatment snapshot has a stale plan digest".into());
            }
            if plan
                .treatments
                .iter()
                .find(|item| item.id == selection.treatment.id)
                != Some(&selection.treatment)
            {
                return Err("selected treatment snapshot differs from the plan".into());
            }
            if self.plan_agent.as_ref() != Some(&selection.planner) {
                return Err("selected treatment planner provenance differs from the plan".into());
            }
        }
        if let Some(candidate) = &self.candidate {
            CandidatePatch {
                summary: candidate.summary.clone(),
                files: candidate.files.clone(),
                verification_commands: Vec::new(),
            }
            .validate()?;
            candidate.verification.validate()?;
            if candidate.base_version != self.accepted.version {
                return Err("restored candidate is stale".into());
            }
            if self.selected_treatment_id.as_deref() != Some(candidate.treatment_id.as_str()) {
                return Err("restored candidate treatment does not match the selection".into());
            }
            if candidate.diff != diff_files(&self.accepted.files, &candidate.files) {
                return Err("restored candidate diff does not match its files".into());
            }
            let mut candidate_files = self.accepted.files.clone();
            for file in &candidate.files {
                candidate_files.insert(file.path.clone(), file.content.clone());
            }
            if candidate.verification.workspace_digest != source_digest(&candidate_files) {
                return Err("restored verification does not match candidate bytes".into());
            }
        }
        match self.phase {
            WorkspacePhase::Described
                if self.treatment_plan.is_none()
                    && self.selected_treatment_id.is_none()
                    && self.selected_treatment.is_none()
                    && self.candidate.is_none() => {}
            WorkspacePhase::TreatmentsReady
                if self.treatment_plan.is_some()
                    && self.selected_treatment_id.is_none()
                    && self.selected_treatment.is_none()
                    && self.candidate.is_none() => {}
            WorkspacePhase::TreatmentSelected
                if self.selected_treatment_id.is_some() && self.candidate.is_none() => {}
            WorkspacePhase::ReviewRequired
                if self
                    .candidate
                    .as_ref()
                    .is_some_and(|item| item.verification.passed) => {}
            WorkspacePhase::Failed
                if self
                    .candidate
                    .as_ref()
                    .is_some_and(|item| !item.verification.passed)
                    || self.failure.is_some() => {}
            WorkspacePhase::Accepted if self.candidate.is_none() => {}
            _ => return Err("restored workspace phase contradicts its contents".into()),
        }
        Ok(())
    }

    pub fn set_plan(&mut self, plan: TreatmentPlan, now: u64) -> Result<(), String> {
        plan.validate()?;
        self.treatment_plan = Some(plan);
        self.plan_agent = None;
        self.selected_treatment_id = None;
        self.selected_treatment = None;
        self.candidate = None;
        self.generation_agent = None;
        self.phase = WorkspacePhase::TreatmentsReady;
        self.updated_at = now;
        Ok(())
    }

    pub fn select(
        &mut self,
        treatment_id: &str,
        refinement: ClinicianRefinement,
        selected_by: &str,
        now: u64,
    ) -> Result<(), String> {
        let plan = self
            .treatment_plan
            .as_ref()
            .ok_or_else(|| "treatment plan is missing".to_string())?;
        let treatment = plan
            .treatments
            .iter()
            .find(|item| item.id == treatment_id)
            .cloned()
            .ok_or_else(|| "selected treatment is not in this plan".to_string())?;
        let planner = self
            .plan_agent
            .clone()
            .ok_or_else(|| "treatment plan provenance is missing".to_string())?;
        refinement.validate()?;
        planner.validate()?;
        validate_bounded_text("selection actor", selected_by, 128)?;
        self.selected_treatment_id = Some(treatment_id.to_string());
        self.selected_treatment = Some(TreatmentSelection {
            treatment,
            refinement,
            plan_digest: treatment_plan_digest(plan),
            planner,
            selected_by: selected_by.to_string(),
            selected_at: now,
        });
        self.candidate = None;
        self.generation_agent = None;
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
        validate_checkpoint_files(&verified_files)?;
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

    fn select(workspace: &mut WorkspaceRecord, treatment_id: &str, now: u64) {
        workspace.plan_agent = Some(AgentProvenance {
            provider: "digitalocean".into(),
            model: "gemma-4-31B-it".into(),
            deployment_version: Some("planner-test-v1".into()),
            fallback_reason: None,
        });
        workspace
            .select(treatment_id, ClinicianRefinement::default(), "dr-test", now)
            .unwrap();
    }

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
    fn treatment_ids_are_bounded_opaque_slugs() {
        for valid in ["guided-worklist", "a", "2-step-view", &"a".repeat(64)] {
            let mut value = plan();
            value.treatments[0].id = valid.to_string();
            value.recommended_treatment_id = valid.to_string();
            assert!(value.validate().is_ok(), "valid id was rejected: {valid:?}");
        }
        for invalid in [
            "",
            "Guided",
            "-leading",
            "has space",
            "has\ttab",
            "x');globalThis.pwned=1;//",
            "quote\"",
            "back\\slash",
            "slash/value",
            "unicode-é",
            &"a".repeat(65),
        ] {
            let mut value = plan();
            value.treatments[0].id = invalid.to_string();
            value.recommended_treatment_id = invalid.to_string();
            assert!(
                value.validate().is_err(),
                "unsafe id was accepted: {invalid:?}"
            );
        }

        let mut value = plan();
        value.treatments[1].id = "x');globalThis.pwned=1;//".into();
        assert!(
            value.validate().is_err(),
            "an unsafe non-recommended treatment id was accepted"
        );
    }

    #[test]
    fn treatment_text_and_refinement_are_bounded_data() {
        let mut value = plan();
        value.treatments[0].label = "x".repeat(101);
        assert!(value.validate().is_err());

        let mut value = plan();
        value.treatments[0].screen_changes = vec!["change".into(); 7];
        assert!(value.validate().is_err());

        let mut value = plan();
        value.treatments[0].user_outcome = "unsafe\ncontrol".into();
        assert!(value.validate().is_err());

        assert!(ClinicianRefinement {
            presentation: PresentationMode::ContextFirst,
            emphasis: Some("</script><script>globalThis.pwned=1</script>${value}".into()),
        }
        .validate()
        .is_ok());
        assert!(ClinicianRefinement {
            presentation: PresentationMode::TaskFirst,
            emphasis: Some("x".repeat(MAX_REFINEMENT_BYTES + 1)),
        }
        .validate()
        .is_err());
        assert!(ClinicianRefinement {
            presentation: PresentationMode::TaskFirst,
            emphasis: Some("two\nparagraphs".into()),
        }
        .validate()
        .is_err());
    }

    #[test]
    fn selection_snapshots_the_plan_and_replanning_clears_it() {
        let mut workspace = WorkspaceRecord::new("app".into(), BTreeMap::new(), 1);
        let first_plan = plan();
        workspace.set_plan(first_plan.clone(), 2).unwrap();
        workspace.plan_agent = Some(AgentProvenance {
            provider: "digitalocean".into(),
            model: "gemma-4-31B-it".into(),
            deployment_version: Some("planner-v1".into()),
            fallback_reason: None,
        });
        let refinement = ClinicianRefinement {
            presentation: PresentationMode::ContextFirst,
            emphasis: Some("Lead with the follow-up explanation.".into()),
        };
        workspace
            .select("daily-view", refinement.clone(), "dr-test", 3)
            .unwrap();
        let selected = workspace.selected_treatment.as_ref().unwrap();
        assert_eq!(selected.treatment, first_plan.treatments[1]);
        assert_eq!(selected.refinement, refinement);
        assert_eq!(selected.plan_digest, treatment_plan_digest(&first_plan));

        let before = workspace.clone();
        assert!(workspace
            .select("missing", ClinicianRefinement::default(), "dr-test", 4)
            .is_err());
        assert_eq!(workspace, before);

        workspace.set_plan(plan(), 5).unwrap();
        assert!(workspace.selected_treatment_id.is_none());
        assert!(workspace.selected_treatment.is_none());
        assert!(workspace.generation_agent.is_none());
    }

    #[test]
    fn failed_or_rejected_candidate_never_changes_the_checkpoint() {
        let mut workspace = WorkspaceRecord::new("app".into(), BTreeMap::new(), 1);
        workspace.set_plan(plan(), 2).unwrap();
        select(&mut workspace, "calm-list", 3);
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
        select(&mut workspace, "calm-list", 3);
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

    #[test]
    fn restored_workspace_revalidates_digest_candidate_and_phase() {
        let mut workspace = WorkspaceRecord::new("app".into(), BTreeMap::new(), 1);
        assert!(workspace.validate_restored().is_ok());

        let mut bad_digest = workspace.clone();
        bad_digest.accepted.digest = "sha256:tampered".into();
        assert!(bad_digest.validate_restored().is_err());

        workspace.set_plan(plan(), 2).unwrap();
        select(&mut workspace, "calm-list", 3);
        workspace
            .review_candidate("good".into(), patch("web/x", "good"), pass(4), 4)
            .unwrap();
        assert!(workspace.validate_restored().is_ok());

        let mut tampered_selection = workspace.clone();
        tampered_selection
            .selected_treatment
            .as_mut()
            .unwrap()
            .treatment
            .label = "Tampered after selection".into();
        assert!(tampered_selection.validate_restored().is_err());

        let mut stale_selection = workspace.clone();
        stale_selection
            .selected_treatment
            .as_mut()
            .unwrap()
            .plan_digest = "sha256:stale".into();
        assert!(stale_selection.validate_restored().is_err());

        let mut stale = workspace.clone();
        stale.candidate.as_mut().unwrap().base_version += 1;
        assert!(stale.validate_restored().is_err());

        let mut wrong_treatment = workspace.clone();
        wrong_treatment.candidate.as_mut().unwrap().treatment_id = "daily-view".into();
        assert!(wrong_treatment.validate_restored().is_err());

        let mut contradictory = workspace;
        contradictory.phase = WorkspacePhase::Accepted;
        assert!(contradictory.validate_restored().is_err());

        let mut unsafe_id = WorkspaceRecord::new("app".into(), BTreeMap::new(), 1);
        let mut unsafe_plan = plan();
        unsafe_plan.treatments[0].id = "x');globalThis.pwned=1;//".into();
        unsafe_plan.recommended_treatment_id = unsafe_plan.treatments[0].id.clone();
        unsafe_id.treatment_plan = Some(unsafe_plan);
        unsafe_id.phase = WorkspacePhase::TreatmentsReady;
        assert!(unsafe_id.validate_restored().is_err());
    }

    #[test]
    fn valid_patch_cannot_grow_the_merged_workspace_past_its_budget() {
        let files = (0..MAX_SOURCE_FILES)
            .map(|index| (format!("web/file-{index}.txt"), "x".to_string()))
            .collect();
        let mut workspace = WorkspaceRecord::new("app".into(), files, 1);
        workspace.set_plan(plan(), 2).unwrap();
        select(&mut workspace, "calm-list", 3);
        let patch = patch("web/one-too-many.txt", "good");
        assert!(patch.validate().is_ok(), "the patch alone is within budget");
        let mut merged = workspace.accepted.files.clone();
        merged.insert("web/one-too-many.txt".into(), "good".into());
        let report = VerificationReport {
            workspace_digest: source_digest(&merged),
            ..pass(4)
        };
        assert!(workspace
            .review_candidate("too-large".into(), patch, report, 4)
            .is_err());
        assert_eq!(workspace.phase, WorkspacePhase::TreatmentSelected);
        assert!(workspace.candidate.is_none());
    }
}
