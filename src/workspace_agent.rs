//! Typed provider boundary for source-workspace planning and generation.
//!
//! Remote agents only propose data. The Rust workspace validates every plan
//! and patch again and remains the sole authority that verifies, checkpoints,
//! accepts, exports, or deploys source.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

use crate::workspace::{CandidateFile, CandidatePatch, Checkpoint, Treatment, TreatmentPlan};

const SCHEMA_VERSION: u8 = 1;
const DEFAULT_TIMEOUT_SECS: u64 = 45;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 600_000;
const MAX_CONFIGURED_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub type AgentFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<AgentOutput<T>, AgentError>> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct PlanRequest {
    pub thread_id: String,
    pub task: String,
    pub pack: String,
    pub workspace_summary: String,
}

#[derive(Clone, Debug)]
pub struct GenerateRequest {
    pub thread_id: String,
    pub task: String,
    pub pack: String,
    pub workspace_summary: String,
    pub selected_treatment_id: String,
    /// Local-only input for the deterministic floor. It is never serialized
    /// into a DigitalOcean request.
    pub accepted_files: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct AgentOutput<T> {
    pub value: T,
    pub provider: &'static str,
    pub fallback: Option<AgentError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentErrorKind {
    Transport,
    Timeout,
    Unauthorized,
    RateLimited,
    Remote4xx,
    Remote5xx,
    Oversized,
    Schema,
    InvalidPlan,
    InvalidCandidate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentError {
    pub kind: AgentErrorKind,
    pub status: Option<u16>,
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(f, "workspace agent {:?} ({status})", self.kind),
            None => write!(f, "workspace agent {:?}", self.kind),
        }
    }
}

impl std::error::Error for AgentError {}

pub trait WorkspaceAgent: Send + Sync {
    fn plan<'a>(&'a self, request: PlanRequest) -> AgentFuture<'a, TreatmentPlan>;
    fn generate<'a>(&'a self, request: GenerateRequest) -> AgentFuture<'a, CandidatePatch>;
}

#[derive(Default)]
pub struct DeterministicWorkspaceAgent;

impl WorkspaceAgent for DeterministicWorkspaceAgent {
    fn plan<'a>(&'a self, request: PlanRequest) -> AgentFuture<'a, TreatmentPlan> {
        Box::pin(async move {
            let plan = deterministic_plan(&request.task);
            plan.validate().map_err(|_| AgentError {
                kind: AgentErrorKind::InvalidPlan,
                status: None,
            })?;
            Ok(AgentOutput {
                value: plan,
                provider: "deterministic",
                fallback: None,
            })
        })
    }

    fn generate<'a>(&'a self, request: GenerateRequest) -> AgentFuture<'a, CandidatePatch> {
        Box::pin(async move {
            let patch = deterministic_patch(&request).map_err(|_| AgentError {
                kind: AgentErrorKind::InvalidCandidate,
                status: None,
            })?;
            patch.validate().map_err(|_| AgentError {
                kind: AgentErrorKind::InvalidCandidate,
                status: None,
            })?;
            Ok(AgentOutput {
                value: patch,
                provider: "deterministic",
                fallback: None,
            })
        })
    }
}

pub struct FallbackWorkspaceAgent {
    primary: Arc<dyn WorkspaceAgent>,
    floor: Arc<dyn WorkspaceAgent>,
}

impl FallbackWorkspaceAgent {
    pub fn new(primary: Arc<dyn WorkspaceAgent>, floor: Arc<dyn WorkspaceAgent>) -> Self {
        Self { primary, floor }
    }
}

impl WorkspaceAgent for FallbackWorkspaceAgent {
    fn plan<'a>(&'a self, request: PlanRequest) -> AgentFuture<'a, TreatmentPlan> {
        Box::pin(async move {
            match self.primary.plan(request.clone()).await {
                Ok(output) => Ok(output),
                Err(error) => {
                    let mut output = self.floor.plan(request).await?;
                    output.fallback = Some(error);
                    Ok(output)
                }
            }
        })
    }

    fn generate<'a>(&'a self, request: GenerateRequest) -> AgentFuture<'a, CandidatePatch> {
        Box::pin(async move {
            match self.primary.generate(request.clone()).await {
                Ok(output) => Ok(output),
                Err(error) => {
                    let mut output = self.floor.generate(request).await?;
                    output.fallback = Some(error);
                    Ok(output)
                }
            }
        })
    }
}

struct AccessKey(String);

impl fmt::Debug for AccessKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccessKey([REDACTED])")
    }
}

pub struct DigitalOceanWorkspaceAgent {
    client: Client,
    planner_endpoint: Url,
    generator_endpoint: Option<Url>,
    access_key: AccessKey,
    max_response_bytes: usize,
}

impl DigitalOceanWorkspaceAgent {
    pub fn new(
        planner_endpoint: &str,
        generator_endpoint: Option<&str>,
        access_key: String,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self> {
        let planner_endpoint = planner_chat_url(planner_endpoint)?;
        validate_private_endpoint(&planner_endpoint, "DIGITALOCEAN_PLANNER_ENDPOINT")?;
        let generator_endpoint = generator_endpoint
            .map(|value| {
                let url =
                    Url::parse(value).context("DIGITALOCEAN_GENERATOR_ENDPOINT is not a URL")?;
                validate_private_endpoint(&url, "DIGITALOCEAN_GENERATOR_ENDPOINT")?;
                Ok::<_, anyhow::Error>(url)
            })
            .transpose()?;
        if access_key.trim().is_empty() {
            bail!("DIGITALOCEAN_PLANNER_ACCESS_KEY is required");
        }
        if timeout.is_zero() || timeout > Duration::from_secs(120) {
            bail!("WORKSPACE_AGENT_TIMEOUT_SECS must be between 1 and 120");
        }
        if !(crate::workspace::MAX_SOURCE_BYTES..=MAX_CONFIGURED_RESPONSE_BYTES)
            .contains(&max_response_bytes)
        {
            bail!("WORKSPACE_AGENT_MAX_RESPONSE_BYTES is outside the safe range");
        }
        let client = Client::builder()
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .timeout(timeout)
            .build()
            .context("building DigitalOcean agent client")?;
        Ok(Self {
            client,
            planner_endpoint,
            generator_endpoint,
            access_key: AccessKey(access_key),
            max_response_bytes,
        })
    }

    async fn post_json<T: Serialize + ?Sized>(
        &self,
        endpoint: Url,
        request: &T,
    ) -> std::result::Result<Vec<u8>, AgentError> {
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(&self.access_key.0)
            .json(request)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        self.read_bounded(response).await
    }

    async fn read_bounded(
        &self,
        response: reqwest::Response,
    ) -> std::result::Result<Vec<u8>, AgentError> {
        let status = response.status();
        if !status.is_success() {
            return Err(AgentError {
                kind: match status.as_u16() {
                    401 | 403 => AgentErrorKind::Unauthorized,
                    429 => AgentErrorKind::RateLimited,
                    400..=499 => AgentErrorKind::Remote4xx,
                    _ => AgentErrorKind::Remote5xx,
                },
                status: Some(status.as_u16()),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.max_response_bytes as u64)
        {
            return Err(AgentError {
                kind: AgentErrorKind::Oversized,
                status: Some(status.as_u16()),
            });
        }
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
            if body.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err(AgentError {
                    kind: AgentErrorKind::Oversized,
                    status: Some(status.as_u16()),
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn invoke_generator<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: Url,
        request: &WireRequest<'_>,
    ) -> std::result::Result<T, AgentError> {
        let body = self.post_json(endpoint, request).await?;
        serde_json::from_slice(&body).map_err(|_| AgentError {
            kind: AgentErrorKind::Schema,
            status: Some(200),
        })
    }
}

fn validate_private_endpoint(endpoint: &Url, name: &str) -> Result<()> {
    let loopback = endpoint
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .map(|ip| ip.is_loopback())
        .unwrap_or(endpoint.host_str() == Some("localhost"));
    if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
        bail!("{name} must use HTTPS (loopback HTTP is allowed for local tests)");
    }
    Ok(())
}

fn planner_chat_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value).context("DIGITALOCEAN_PLANNER_ENDPOINT is not a URL")?;
    if !url.path().ends_with("/api/v1/chat/completions") {
        url.set_path("/api/v1/chat/completions");
    }
    Ok(url)
}

impl WorkspaceAgent for DigitalOceanWorkspaceAgent {
    fn plan<'a>(&'a self, request: PlanRequest) -> AgentFuture<'a, TreatmentPlan> {
        Box::pin(async move {
            let wire = WireRequest {
                schema_version: SCHEMA_VERSION,
                action: "plan",
                thread_id: &request.thread_id,
                task: &request.task,
                pack: &request.pack,
                workspace_summary: &request.workspace_summary,
                selected_treatment_id: None,
            };
            let user_content = serde_json::to_string(&wire).map_err(|_| schema_error())?;
            let chat = ChatRequest {
                // Managed DigitalOcean agents reject system and developer
                // messages. Their trusted instructions live in the agent
                // configuration, so the request contains untrusted user data
                // only.
                messages: vec![ChatMessage {
                    role: "user",
                    content: &user_content,
                }],
                stream: false,
            };
            let body = self.post_json(self.planner_endpoint.clone(), &chat).await?;
            let wrapper: ChatCompletion =
                serde_json::from_slice(&body).map_err(|_| schema_error())?;
            let content = wrapper
                .choices
                .first()
                .map(|choice| choice.message.content.as_str())
                .ok_or_else(schema_error)?;
            let envelope: PlanEnvelope =
                serde_json::from_str(content).map_err(|_| schema_error())?;
            if envelope.schema_version != SCHEMA_VERSION {
                return Err(schema_error());
            }
            let plan = envelope.treatment_plan;
            plan.validate().map_err(|_| AgentError {
                kind: AgentErrorKind::InvalidPlan,
                status: None,
            })?;
            Ok(AgentOutput {
                value: plan,
                provider: "digitalocean",
                fallback: None,
            })
        })
    }

    fn generate<'a>(&'a self, request: GenerateRequest) -> AgentFuture<'a, CandidatePatch> {
        Box::pin(async move {
            let Some(endpoint) = self.generator_endpoint.clone() else {
                let mut output = DeterministicWorkspaceAgent.generate(request).await?;
                output.provider = "deterministic";
                return Ok(output);
            };
            let wire = WireRequest {
                schema_version: SCHEMA_VERSION,
                action: "generate",
                thread_id: &request.thread_id,
                task: &request.task,
                pack: &request.pack,
                workspace_summary: &request.workspace_summary,
                selected_treatment_id: Some(&request.selected_treatment_id),
            };
            let envelope: GenerateEnvelope = self.invoke_generator(endpoint, &wire).await?;
            if envelope.schema_version != SCHEMA_VERSION {
                return Err(schema_error());
            }
            envelope
                .candidate_patch
                .validate()
                .map_err(|_| AgentError {
                    kind: AgentErrorKind::InvalidCandidate,
                    status: None,
                })?;
            Ok(AgentOutput {
                value: envelope.candidate_patch,
                provider: "digitalocean",
                fallback: None,
            })
        })
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletion {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatAssistantMessage,
}

#[derive(Deserialize)]
struct ChatAssistantMessage {
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanEnvelope {
    schema_version: u8,
    treatment_plan: TreatmentPlan,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct WireRequest<'a> {
    schema_version: u8,
    action: &'static str,
    thread_id: &'a str,
    task: &'a str,
    pack: &'a str,
    workspace_summary: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_treatment_id: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateEnvelope {
    schema_version: u8,
    candidate_patch: CandidatePatch,
}

fn schema_error() -> AgentError {
    AgentError {
        kind: AgentErrorKind::Schema,
        status: None,
    }
}

fn map_reqwest_error(error: reqwest::Error) -> AgentError {
    AgentError {
        kind: if error.is_timeout() {
            AgentErrorKind::Timeout
        } else {
            AgentErrorKind::Transport
        },
        status: error.status().map(|status| status.as_u16()),
    }
}

pub fn from_env() -> Result<Arc<dyn WorkspaceAgent>> {
    let provider =
        std::env::var("WORKSPACE_AGENT_PROVIDER").unwrap_or_else(|_| "deterministic".into());
    match provider.trim() {
        "" | "deterministic" => Ok(Arc::new(DeterministicWorkspaceAgent)),
        "digitalocean" => {
            let planner_endpoint = std::env::var("DIGITALOCEAN_PLANNER_ENDPOINT")
                .context("DIGITALOCEAN_PLANNER_ENDPOINT is required")?;
            let access_key = std::env::var("DIGITALOCEAN_PLANNER_ACCESS_KEY")
                .context("DIGITALOCEAN_PLANNER_ACCESS_KEY is required")?;
            let generator_endpoint = std::env::var("DIGITALOCEAN_GENERATOR_ENDPOINT")
                .ok()
                .filter(|value| !value.trim().is_empty());
            let timeout =
                parse_bounded_env("WORKSPACE_AGENT_TIMEOUT_SECS", DEFAULT_TIMEOUT_SECS, 1, 120)?;
            let max = parse_bounded_env(
                "WORKSPACE_AGENT_MAX_RESPONSE_BYTES",
                DEFAULT_MAX_RESPONSE_BYTES as u64,
                crate::workspace::MAX_SOURCE_BYTES as u64,
                MAX_CONFIGURED_RESPONSE_BYTES as u64,
            )? as usize;
            let primary = Arc::new(DigitalOceanWorkspaceAgent::new(
                &planner_endpoint,
                generator_endpoint.as_deref(),
                access_key,
                Duration::from_secs(timeout),
                max,
            )?);
            Ok(Arc::new(FallbackWorkspaceAgent::new(
                primary,
                Arc::new(DeterministicWorkspaceAgent),
            )))
        }
        other => bail!("unsupported WORKSPACE_AGENT_PROVIDER {other:?}"),
    }
}

fn parse_bounded_env(name: &str, default: u64, min: u64, max: u64) -> Result<u64> {
    let value = match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .with_context(|| format!("{name} must be an integer"))?,
        Err(_) => default,
    };
    if !(min..=max).contains(&value) {
        bail!("{name} must be between {min} and {max}");
    }
    Ok(value)
}

pub fn workspace_summary(checkpoint: &Checkpoint) -> String {
    let paths = checkpoint
        .files
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "checkpoint={} digest={} paths={}",
        checkpoint.version, checkpoint.digest, paths
    )
}

fn deterministic_plan(task: &str) -> TreatmentPlan {
    TreatmentPlan {
        problem: task.trim().to_string(),
        recommended_treatment_id: "guided-worklist".into(),
        treatments: vec![
            Treatment {
                id: "guided-worklist".into(),
                label: "Guided worklist".into(),
                user_outcome: "See the next safe action without scanning the whole record".into(),
                screen_changes: vec![
                    "A calm priority list with one clear next action".into(),
                    "A visible reason for every flagged item".into(),
                ],
                data_changes: vec!["Keep status and review notes with each synthetic item".into()],
                safety_notes: vec!["Keep escalation and synthetic-data labels visible".into()],
            },
            Treatment {
                id: "patient-timeline".into(),
                label: "Patient timeline".into(),
                user_outcome: "Understand what changed before deciding what to do next".into(),
                screen_changes: vec![
                    "A chronological event view".into(),
                    "Filters for unresolved and reviewed events".into(),
                ],
                data_changes: vec!["Record events as append-only synthetic entries".into()],
                safety_notes: vec!["Do not turn the timeline into clinical advice".into()],
            },
            Treatment {
                id: "focused-form".into(),
                label: "Focused form".into(),
                user_outcome: "Complete one task with fewer fields and a clear confirmation".into(),
                screen_changes: vec![
                    "A short step-by-step form".into(),
                    "A review screen before saving".into(),
                ],
                data_changes: vec!["Save only the fields needed for this synthetic workflow".into()],
                safety_notes: vec!["Show what is saved and who must review it".into()],
            },
        ],
        acceptance_checks: vec![
            "The chosen workflow is usable with a keyboard".into(),
            "Synthetic-data and safety limits remain visible".into(),
            "The Svelte client and Rust server remain independently testable".into(),
        ],
    }
}

fn deterministic_patch(request: &GenerateRequest) -> Result<CandidatePatch> {
    let current = request
        .accepted_files
        .get("web/src/routes/+page.svelte")
        .context("accepted Svelte page is missing")?;
    let safe_task = request.task.trim().replace(['<', '>'], "");
    let addition = format!(
        "\n<section class=\"hc-card hc-stack\" data-treatment=\"{}\"><h2>Selected treatment</h2><p>{}</p><p class=\"hc-help\">Synthetic examples only. Review this change before accepting it.</p></section>\n",
        request.selected_treatment_id, safe_task
    );
    let content = current.replacen("</main>", &format!("{addition}</main>"), 1);
    Ok(CandidatePatch {
        summary: format!(
            "Apply the {} treatment to the Svelte screen",
            request.selected_treatment_id
        ),
        files: vec![CandidateFile {
            path: "web/src/routes/+page.svelte".into(),
            content,
            reason: "This is the screen treatment the user selected".into(),
        }],
        verification_commands: vec![
            "npm run check".into(),
            "npm run build".into(),
            "cargo test --manifest-path server/Cargo.toml".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn serve_once(response_body: String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, received) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..headers_end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= headers_end + 4 + length {
                        break;
                    }
                }
            }
            let _ = sent.send(String::from_utf8(request).unwrap());
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
        });
        (format!("http://{address}"), received)
    }

    #[test]
    fn access_key_debug_is_redacted() {
        let rendered = format!("{:?}", AccessKey("super-secret".into()));
        assert_eq!(rendered, "AccessKey([REDACTED])");
        assert!(!rendered.contains("super-secret"));
    }

    #[test]
    fn rejects_public_plain_http() {
        let error = DigitalOceanWorkspaceAgent::new(
            "http://example.com/agent",
            None,
            "key".into(),
            Duration::from_secs(5),
            DEFAULT_MAX_RESPONSE_BYTES,
        )
        .err()
        .expect("public HTTP must be rejected");
        assert!(error.to_string().contains("HTTPS"));
    }

    #[test]
    fn summary_contains_no_source_content() {
        let checkpoint = Checkpoint::new(
            2,
            BTreeMap::from([("web/private.txt".into(), "sentinel-phi".into())]),
            1,
        );
        let summary = workspace_summary(&checkpoint);
        assert!(summary.contains("web/private.txt"));
        assert!(!summary.contains("sentinel-phi"));
    }

    #[tokio::test]
    async fn plan_is_v1_bearer_request_without_selected_treatment() {
        let plan = r#"{"schema_version":1,"treatment_plan":{"problem":"queue","recommended_treatment_id":"a","treatments":[{"id":"a","label":"A","user_outcome":"A","screen_changes":["A"]},{"id":"b","label":"B","user_outcome":"B","screen_changes":["B"]}],"acceptance_checks":["works"]}}"#;
        let body = serde_json::json!({
            "id": "completion-1",
            "choices": [{"message": {"role": "assistant", "content": plan}}]
        })
        .to_string();
        let (endpoint, request) = serve_once(body);
        let agent = DigitalOceanWorkspaceAgent::new(
            &endpoint,
            None,
            "private-key".into(),
            Duration::from_secs(5),
            DEFAULT_MAX_RESPONSE_BYTES,
        )
        .unwrap();
        let output = agent
            .plan(PlanRequest {
                thread_id: "app-1".into(),
                task: "queue".into(),
                pack: "intake".into(),
                workspace_summary: "checkpoint=0".into(),
            })
            .await
            .unwrap();
        assert_eq!(output.provider, "digitalocean");
        let raw = request.recv().unwrap();
        let lower = raw.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer private-key"));
        let json = raw.split("\r\n\r\n").nth(1).unwrap();
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(value["stream"], false);
        assert_eq!(value["messages"].as_array().unwrap().len(), 1);
        assert_eq!(value["messages"][0]["role"], "user");
        let inner: serde_json::Value =
            serde_json::from_str(value["messages"][0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(inner["schema_version"], 1);
        assert_eq!(inner["action"], "plan");
        assert!(inner.get("selected_treatment_id").is_none());
        assert!(raw.starts_with("POST /api/v1/chat/completions "));
    }

    #[tokio::test]
    async fn generate_sends_selected_treatment_but_not_local_files() {
        let body = r#"{"schema_version":1,"candidate_patch":{"summary":"change","files":[{"path":"web/src/routes/+page.svelte","content":"<p>safe</p>","reason":"screen"}],"verification_commands":["npm run check"]}}"#.to_string();
        let (endpoint, request) = serve_once(body);
        let agent = DigitalOceanWorkspaceAgent::new(
            &endpoint,
            Some(&endpoint),
            "private-key".into(),
            Duration::from_secs(5),
            DEFAULT_MAX_RESPONSE_BYTES,
        )
        .unwrap();
        agent
            .generate(GenerateRequest {
                thread_id: "app-1".into(),
                task: "queue".into(),
                pack: "intake".into(),
                workspace_summary: "checkpoint=0".into(),
                selected_treatment_id: "timeline".into(),
                accepted_files: BTreeMap::from([(
                    "web/private".into(),
                    "sentinel-source-secret".into(),
                )]),
            })
            .await
            .unwrap();
        let raw = request.recv().unwrap();
        let json = raw.split("\r\n\r\n").nth(1).unwrap();
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(value["selected_treatment_id"], "timeline");
        assert!(!json.contains("sentinel-source-secret"));
        assert!(value.get("accepted_files").is_none());
    }

    #[tokio::test]
    async fn strict_envelope_rejects_unknown_fields() {
        let invalid_plan = r#"{"schema_version":1,"treatment_plan":{"problem":"queue","unexpected":true,"recommended_treatment_id":"a","treatments":[{"id":"a","label":"A","user_outcome":"A","screen_changes":["A"]},{"id":"b","label":"B","user_outcome":"B","screen_changes":["B"]}],"acceptance_checks":["works"]}}"#;
        let body = serde_json::json!({
            "choices": [{"message": {"content": invalid_plan}}]
        })
        .to_string();
        let (endpoint, _) = serve_once(body);
        let agent = DigitalOceanWorkspaceAgent::new(
            &endpoint,
            None,
            "private-key".into(),
            Duration::from_secs(5),
            DEFAULT_MAX_RESPONSE_BYTES,
        )
        .unwrap();
        let error = agent
            .plan(PlanRequest {
                thread_id: "app-1".into(),
                task: "queue".into(),
                pack: "intake".into(),
                workspace_summary: "checkpoint=0".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.kind, AgentErrorKind::Schema);
        assert!(!error.to_string().contains("unexpected"));
    }

    #[tokio::test]
    async fn generate_is_deterministic_when_no_worker_endpoint_is_configured() {
        let agent = DigitalOceanWorkspaceAgent::new(
            "http://127.0.0.1:9",
            None,
            "private-key".into(),
            Duration::from_secs(1),
            DEFAULT_MAX_RESPONSE_BYTES,
        )
        .unwrap();
        let output = agent
            .generate(GenerateRequest {
                thread_id: "app-1".into(),
                task: "add a queue".into(),
                pack: "intake".into(),
                workspace_summary: "checkpoint=0".into(),
                selected_treatment_id: "guided-worklist".into(),
                accepted_files: BTreeMap::from([(
                    "web/src/routes/+page.svelte".into(),
                    "<main></main>".into(),
                )]),
            })
            .await
            .unwrap();
        assert_eq!(output.provider, "deterministic");
        assert!(output.fallback.is_none());
        assert!(output.value.files[0].content.contains("add a queue"));
    }
}
