//! Executable, fail-closed verification for agent-proposed source workspaces.
//!
//! The model's `verification_commands` are data for the review UI only. This
//! module never reads them. A platform-owned profile materializes the exact
//! accepted checkpoint plus candidate overlay and runs a fixed check sequence.

use crate::workspace::{
    source_digest, CandidatePatch, CheckStatus, Checkpoint, VerificationCheck, VerificationReport,
    EXECUTABLE_CHECK_IDS,
};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::{Read, Write};
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::{Builder, TempDir};
use tokio::process::Command;

pub const CHECK_IDS: [&str; 5] = EXECUTABLE_CHECK_IDS;
const MAX_REPORT_BYTES: u64 = 64 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 180;
const MAX_TIMEOUT_SECS: u64 = 300;
const DETERMINISTIC_PROFILE: &str = "practice-studio-deterministic-verifier-v1";

#[derive(Clone)]
pub struct VerifyRequest {
    pub accepted: Checkpoint,
    pub candidate: CandidatePatch,
}

pub type VerifyFuture<'a> = Pin<Box<dyn Future<Output = VerificationReport> + Send + 'a>>;

pub trait WorkspaceVerifier: Send + Sync {
    fn verify<'a>(&'a self, request: VerifyRequest) -> VerifyFuture<'a>;
}

pub fn from_env() -> Result<Arc<dyn WorkspaceVerifier>> {
    match std::env::var("WORKSPACE_VERIFIER_IMAGE")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        Some(image) => Ok(Arc::new(OciWorkspaceVerifier::from_env(image)?)),
        None => Ok(Arc::new(DeterministicWorkspaceVerifier)),
    }
}

#[derive(Default)]
pub struct DeterministicWorkspaceVerifier;

impl WorkspaceVerifier for DeterministicWorkspaceVerifier {
    fn verify<'a>(&'a self, request: VerifyRequest) -> VerifyFuture<'a> {
        Box::pin(async move {
            let profile_digest = digest(DETERMINISTIC_PROFILE.as_bytes());
            let (files, workspace_digest) = merged_files(&request);
            let mut checks =
                fixed_checks(CheckStatus::Pass, "verified by deterministic test profile");
            match materialize(&files) {
                Ok(materialized) => {
                    let required = [
                        "web/package.json",
                        "web/src/routes/+page.svelte",
                        "server/Cargo.toml",
                    ];
                    if let Some(path) = required
                        .iter()
                        .find(|path| !materialized.path().join(path).is_file())
                    {
                        fail_from(&mut checks, 0, &format!("required file is missing: {path}"));
                    } else {
                        let page = files
                            .get("web/src/routes/+page.svelte")
                            .map(String::as_str)
                            .unwrap_or("");
                        if !page.contains("$state") {
                            fail_from(&mut checks, 1, "Svelte 5 rune `$state` is missing");
                        } else if !page.to_ascii_lowercase().contains("synthetic") {
                            fail_from(&mut checks, 4, "synthetic-data warning is not visible");
                        }
                    }
                    drop(materialized);
                }
                Err(_) => fail_from(&mut checks, 0, "workspace materialization failed"),
            }
            report(profile_digest, workspace_digest, checks)
        })
    }
}

pub struct OciWorkspaceVerifier {
    runtime: String,
    image: String,
    timeout: Duration,
    profile_digest: String,
}

impl OciWorkspaceVerifier {
    fn from_env(image: String) -> Result<Self> {
        if !image.contains("@sha256:") {
            bail!("WORKSPACE_VERIFIER_IMAGE must be pinned by sha256 digest");
        }
        let runtime =
            std::env::var("WORKSPACE_VERIFIER_RUNTIME").unwrap_or_else(|_| "docker".into());
        if runtime.is_empty()
            || runtime
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        {
            bail!("WORKSPACE_VERIFIER_RUNTIME must be one executable name");
        }
        let timeout_secs = std::env::var("WORKSPACE_VERIFIER_TIMEOUT_SECS")
            .ok()
            .map(|value| value.parse::<u64>())
            .transpose()
            .context("WORKSPACE_VERIFIER_TIMEOUT_SECS is not an integer")?
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        if !(1..=MAX_TIMEOUT_SECS).contains(&timeout_secs) {
            bail!("WORKSPACE_VERIFIER_TIMEOUT_SECS must be between 1 and {MAX_TIMEOUT_SECS}");
        }
        let profile_digest = digest(format!("oci-v1\0{image}").as_bytes());
        Ok(Self {
            runtime,
            image,
            timeout: Duration::from_secs(timeout_secs),
            profile_digest,
        })
    }
}

impl WorkspaceVerifier for OciWorkspaceVerifier {
    fn verify<'a>(&'a self, request: VerifyRequest) -> VerifyFuture<'a> {
        Box::pin(async move {
            let (files, workspace_digest) = merged_files(&request);
            let materialized = match materialize(&files) {
                Ok(value) => value,
                Err(_) => {
                    let mut checks = fixed_checks(CheckStatus::Fail, "not run");
                    checks[0].detail = "workspace materialization failed".into();
                    return report(self.profile_digest.clone(), workspace_digest, checks);
                }
            };
            let report_path = materialized.path().join(".practice-verifier-report.json");
            let user = container_user(materialized.path());
            let container_name = format!(
                "practice-verify-{}-{}",
                &report_id(&self.profile_digest, &workspace_digest)[10..],
                runtime_nonce()
            );
            let mount = match materialized.path().to_str() {
                Some(path) => format!("{path}:/workspace:rw"),
                None => {
                    return failed_report(
                        self.profile_digest.clone(),
                        workspace_digest,
                        "workspace path is not UTF-8",
                    )
                }
            };
            let mut child = match Command::new(&self.runtime)
                .kill_on_drop(true)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .args([
                    "run",
                    "--rm",
                    "--name",
                    &container_name,
                    "--network",
                    "none",
                    "--read-only",
                    "--cpus",
                    "1",
                    "--memory",
                    "1536m",
                    "--memory-swap",
                    "1536m",
                    "--pids-limit",
                    "256",
                    "--cap-drop",
                    "ALL",
                    "--security-opt",
                    "no-new-privileges",
                    "--user",
                    &user,
                    "--tmpfs",
                    "/tmp:rw,nosuid,nodev,noexec,size=128m",
                    "-v",
                    &mount,
                    &self.image,
                    "/opt/practice-studio/verify",
                    "--workspace",
                    "/workspace",
                    "--report",
                    "/workspace/.practice-verifier-report.json",
                ])
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    return failed_report(
                        self.profile_digest.clone(),
                        workspace_digest,
                        &format!("verifier runtime could not start: {error}"),
                    )
                }
            };
            let status = match tokio::time::timeout(self.timeout, child.wait()).await {
                Ok(Ok(status)) => status,
                Ok(Err(error)) => {
                    return failed_report(
                        self.profile_digest.clone(),
                        workspace_digest,
                        &format!("verifier runtime failed: {error}"),
                    )
                }
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    let _ = tokio::time::timeout(
                        Duration::from_secs(5),
                        Command::new(&self.runtime)
                            .args(["rm", "-f", &container_name])
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status(),
                    )
                    .await;
                    return failed_report(
                        self.profile_digest.clone(),
                        workspace_digest,
                        "verifier timed out and was killed",
                    );
                }
            };
            if !status.success() {
                return failed_report(
                    self.profile_digest.clone(),
                    workspace_digest,
                    &format!(
                        "verifier exited with {}",
                        status
                            .code()
                            .map_or_else(|| "a signal".into(), |code| code.to_string())
                    ),
                );
            }
            let checks = match read_oci_report(&report_path) {
                Ok(checks) => checks,
                Err(error) => {
                    return failed_report(
                        self.profile_digest.clone(),
                        workspace_digest,
                        &error.to_string(),
                    )
                }
            };
            report(self.profile_digest.clone(), workspace_digest, checks)
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OciReport {
    checks: Vec<OciCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OciCheck {
    id: String,
    passed: bool,
    detail: String,
}

fn read_oci_report(path: &Path) -> Result<Vec<VerificationCheck>> {
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .context("verifier report is missing")?;
    if file.metadata()?.len() > MAX_REPORT_BYTES {
        bail!("verifier report exceeds {MAX_REPORT_BYTES} bytes");
    }
    let mut raw = String::new();
    file.take(MAX_REPORT_BYTES + 1).read_to_string(&mut raw)?;
    let parsed: OciReport = serde_json::from_str(&raw).context("verifier report is invalid")?;
    if parsed.checks.len() != CHECK_IDS.len()
        || parsed
            .checks
            .iter()
            .zip(CHECK_IDS)
            .any(|(check, id)| check.id != id)
    {
        bail!("verifier report check ids or order are invalid");
    }
    Ok(parsed
        .checks
        .into_iter()
        .map(|check| VerificationCheck {
            id: check.id,
            status: if check.passed {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            },
            detail: bounded_detail(&check.detail),
        })
        .collect())
}

fn merged_files(request: &VerifyRequest) -> (BTreeMap<String, String>, String) {
    let mut files = request.accepted.files.clone();
    for file in &request.candidate.files {
        files.insert(file.path.clone(), file.content.clone());
    }
    let workspace_digest = source_digest(&files);
    (files, workspace_digest)
}

fn materialize(files: &BTreeMap<String, String>) -> Result<TempDir> {
    let temp = Builder::new().prefix("practice-workspace-").tempdir()?;
    for (relative, content) in files {
        validate_relative_path(relative)?;
        let path = temp.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(content.as_bytes())?;
        file.sync_data()?;
    }
    Ok(temp)
}

fn validate_relative_path(path: &str) -> Result<()> {
    if path.is_empty()
        || Path::new(path).is_absolute()
        || path.contains('\\')
        || path.contains('\0')
        || Path::new(path).components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || Path::new(path).components().any(|part| {
            matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Prefix(_)
            )
        })
        || Path::new(path).file_name() == Some(OsStr::new(".practice-verifier-report.json"))
    {
        bail!("unsafe workspace path: {path}");
    }
    Ok(())
}

fn report(
    profile_digest: String,
    workspace_digest: String,
    checks: Vec<VerificationCheck>,
) -> VerificationReport {
    let id = report_id(&profile_digest, &workspace_digest);
    let passed = checks.iter().all(|check| check.status == CheckStatus::Pass);
    VerificationReport {
        id,
        workspace_digest,
        profile_digest,
        checks,
        passed,
        verified_at: now_unix(),
    }
}

fn failed_report(
    profile_digest: String,
    workspace_digest: String,
    detail: &str,
) -> VerificationReport {
    let mut checks = fixed_checks(
        CheckStatus::Fail,
        "not run because verifier infrastructure failed",
    );
    checks[0].detail = bounded_detail(detail);
    report(profile_digest, workspace_digest, checks)
}

fn fixed_checks(status: CheckStatus, detail: &str) -> Vec<VerificationCheck> {
    CHECK_IDS
        .iter()
        .map(|id| VerificationCheck {
            id: (*id).into(),
            status,
            detail: detail.into(),
        })
        .collect()
}

fn fail_from(checks: &mut [VerificationCheck], index: usize, detail: &str) {
    for (offset, check) in checks.iter_mut().enumerate().skip(index) {
        check.status = CheckStatus::Fail;
        check.detail = if offset == index {
            bounded_detail(detail)
        } else {
            format!("not run: prerequisite {} failed", CHECK_IDS[index])
        };
    }
}

fn report_id(profile_digest: &str, workspace_digest: &str) -> String {
    let value = digest(format!("verify-v1\0{profile_digest}\0{workspace_digest}").as_bytes());
    format!("verify-v1-{}", &value[7..31])
}

fn digest(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

fn bounded_detail(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(1024)
        .collect()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn runtime_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(unix)]
fn container_user(path: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    path.metadata()
        .map(|metadata| format!("{}:{}", metadata.uid(), metadata.gid()))
        .unwrap_or_else(|_| "65532:65532".into())
}

#[cfg(not(unix))]
fn container_user(_path: &Path) -> String {
    "65532:65532".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::CandidateFile;

    fn request(command: &str) -> VerifyRequest {
        let accepted = Checkpoint::new(
            0,
            BTreeMap::from([
                ("web/package.json".into(), "{}".into()),
                (
                    "web/src/routes/+page.svelte".into(),
                    "<script>let x = $state(1)</script><p>Synthetic examples only</p>".into(),
                ),
                (
                    "server/Cargo.toml".into(),
                    "[package]\nname='x'\nversion='0.1.0'".into(),
                ),
            ]),
            1,
        );
        VerifyRequest {
            accepted,
            candidate: CandidatePatch {
                summary: "change".into(),
                files: vec![CandidateFile {
                    path: "web/src/routes/+page.svelte".into(),
                    content: "<script>let x = $state(2)</script><p>Synthetic examples only</p>"
                        .into(),
                    reason: "selected".into(),
                }],
                verification_commands: vec![command.into()],
            },
        }
    }

    #[tokio::test]
    async fn report_identity_and_fixed_order_are_deterministic_and_commands_are_ignored() {
        let verifier = DeterministicWorkspaceVerifier;
        let first = verifier.verify(request("rm -rf /")).await;
        let second = verifier.verify(request("echo different")).await;
        assert!(first.passed);
        assert_eq!(first.id, second.id);
        assert_eq!(first.workspace_digest, second.workspace_digest);
        assert_eq!(
            first
                .checks
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            CHECK_IDS
        );
    }

    #[test]
    fn materializer_rejects_traversal_and_cleans_up_on_drop() {
        assert!(materialize(&BTreeMap::from([("../secret".into(), "x".into())])).is_err());
        let temp = materialize(&BTreeMap::from([("web/x".into(), "x".into())])).unwrap();
        let path = temp.path().to_path_buf();
        assert!(path.join("web/x").is_file());
        drop(temp);
        assert!(!path.exists());
    }
}
