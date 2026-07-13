//! Typed provider boundary for source-workspace planning.
//!
//! Remote agents only propose data. The Rust workspace validates every plan
//! and remains the sole authority that generates, verifies, checkpoints,
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

use crate::packs::PackManifest;
use crate::workspace::{
    CandidateFile, CandidatePatch, Checkpoint, Treatment, TreatmentPlan, TreatmentSelection,
};

const SCHEMA_VERSION: u8 = 1;
const DEFAULT_TIMEOUT_SECS: u64 = 45;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 600_000;
const MAX_CONFIGURED_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const DETERMINISTIC_MODEL: &str = "convention-floor-v1";
const DETERMINISTIC_MATERIALIZER: &str = "rust-convention-v2";
const DIGITALOCEAN_PLANNER_MODEL: &str = "gemma-4-31B-it";

pub type AgentFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<AgentOutput<T>, AgentError>> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct PlanRequest {
    pub thread_id: String,
    pub task: String,
    pub pack: String,
    pub workspace_summary: String,
    pub pack_context: PackPlanningContext,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackPlanningContext {
    pub name: String,
    pub description: String,
    pub profile: String,
    pub existing_capabilities: Vec<String>,
    pub treatment_recipes: Vec<Treatment>,
    pub acceptance_checks: Vec<String>,
}

impl PackPlanningContext {
    pub fn from_pack(pack: &PackManifest) -> Self {
        let recipe = |id: &str, label: &str, outcome: String, screen: Vec<&str>| {
            Treatment {
            id: id.into(),
            label: label.into(),
            user_outcome: outcome,
            screen_changes: screen.into_iter().map(str::to_string).collect(),
            data_changes: vec![
                "Reuse the signed pack's existing synthetic fields; do not invent a new clinical schema"
                    .into(),
            ],
            safety_notes: vec![
                "Keep the synthetic-data boundary and the required human review visible".into(),
            ],
        }
        };
        let recipes = pack
            .treatment_recipes
            .iter()
            .map(|id| match id.as_str() {
                "guided-worklist" => recipe(
                    id,
                    "Guided worklist",
                    format!(
                        "See the next safe action in {} without scanning every synthetic record",
                        pack.name
                    ),
                    vec![
                        "A priority worklist built from the pack's existing capabilities",
                        "A visible reason and next action for every synthetic item",
                    ],
                ),
                "event-timeline" => recipe(
                    id,
                    "Event timeline",
                    format!(
                        "Understand what changed in {} before choosing the next action",
                        pack.name
                    ),
                    vec![
                        "A chronological view of the pack's existing synthetic events",
                        "Filters for unresolved and reviewed events",
                    ],
                ),
                "focused-task" => recipe(
                    id,
                    "Focused task",
                    format!("Complete one {} task with a clear review step", pack.name),
                    vec![
                        "One focused task using the pack's existing workflow",
                        "A review screen before the synthetic action is saved",
                    ],
                ),
                _ => unreachable!("trusted pack recipe ids are validated at registry load"),
            })
            .collect();
        Self {
            name: pack.name.clone(),
            description: pack.description.clone(),
            profile: pack.profile.clone(),
            existing_capabilities: pack.scaffold.clone(),
            treatment_recipes: recipes,
            acceptance_checks: vec![
                "The selected recipe is visible in the Svelte workspace".into(),
                "The signed pack browser journey still passes".into(),
                "Synthetic-data and safety limits remain visible".into(),
            ],
        }
    }

    fn plan(&self, task: &str, recommended_treatment_id: &str) -> Result<TreatmentPlan> {
        if !self
            .treatment_recipes
            .iter()
            .any(|recipe| recipe.id == recommended_treatment_id)
        {
            bail!("recommended treatment is not a signed pack recipe");
        }
        let plan = TreatmentPlan {
            problem: bounded_problem(task),
            recommended_treatment_id: recommended_treatment_id.into(),
            treatments: self.treatment_recipes.clone(),
            acceptance_checks: self.acceptance_checks.clone(),
        };
        plan.validate().map_err(anyhow::Error::msg)?;
        Ok(plan)
    }

    fn ground_legacy(&self, task: &str, proposed: &TreatmentPlan) -> Result<TreatmentPlan> {
        let proposed_ids = proposed
            .treatments
            .iter()
            .map(|treatment| treatment.id.as_str())
            .collect::<Vec<_>>();
        let allowed_ids = self
            .treatment_recipes
            .iter()
            .map(|treatment| treatment.id.as_str())
            .collect::<Vec<_>>();
        if proposed_ids != allowed_ids {
            bail!("treatments do not match the signed pack recipes");
        }
        self.plan(task, &proposed.recommended_treatment_id)
    }
}

#[derive(Clone, Debug)]
pub struct GenerateRequest {
    pub thread_id: String,
    pub pack: String,
    pub workspace_summary: String,
    pub selected_treatment: TreatmentSelection,
    /// Exact workflow items shown by both Studio and the exported Svelte app.
    /// Rust snapshots them into the candidate configuration so they are
    /// covered by the accepted checkpoint digest.
    pub features: Vec<String>,
    /// Local-only input for the deterministic floor. It is never serialized
    /// into a DigitalOcean request.
    pub accepted_files: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct AgentOutput<T> {
    pub value: T,
    pub provider: &'static str,
    pub model: &'static str,
    pub deployment_version: Option<String>,
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
            let recommended = request
                .pack_context
                .treatment_recipes
                .first()
                .map(|recipe| recipe.id.as_str())
                .ok_or(AgentError {
                    kind: AgentErrorKind::InvalidPlan,
                    status: None,
                })?;
            let plan = request
                .pack_context
                .plan(&request.task, recommended)
                .map_err(|_| AgentError {
                    kind: AgentErrorKind::InvalidPlan,
                    status: None,
                })?;
            plan.validate().map_err(|_| AgentError {
                kind: AgentErrorKind::InvalidPlan,
                status: None,
            })?;
            Ok(AgentOutput {
                value: plan,
                provider: "deterministic",
                model: DETERMINISTIC_MODEL,
                deployment_version: None,
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
                provider: "rust",
                model: DETERMINISTIC_MATERIALIZER,
                deployment_version: None,
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
    planner_access_key: AccessKey,
    planner_version: Option<String>,
    max_response_bytes: usize,
}

impl DigitalOceanWorkspaceAgent {
    pub fn new(
        planner_endpoint: &str,
        planner_access_key: String,
        planner_version: Option<String>,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self> {
        let planner_endpoint = planner_chat_url(planner_endpoint)?;
        validate_private_endpoint(&planner_endpoint, "DIGITALOCEAN_PLANNER_ENDPOINT")?;
        if planner_access_key.trim().is_empty() {
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
            planner_access_key: AccessKey(planner_access_key),
            planner_version,
            max_response_bytes,
        })
    }

    async fn post_json<T: Serialize + ?Sized>(
        &self,
        endpoint: Url,
        access_key: &AccessKey,
        request: &T,
    ) -> std::result::Result<Vec<u8>, AgentError> {
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(&access_key.0)
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
                pack_context: &request.pack_context,
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
            let body = self
                .post_json(
                    self.planner_endpoint.clone(),
                    &self.planner_access_key,
                    &chat,
                )
                .await?;
            let wrapper: ChatCompletion =
                serde_json::from_slice(&body).map_err(|_| schema_error())?;
            if wrapper.model != DIGITALOCEAN_PLANNER_MODEL {
                return Err(schema_error());
            }
            let content = wrapper
                .choices
                .first()
                .map(|choice| choice.message.content.as_str())
                .ok_or_else(schema_error)?;
            let envelope: PlanEnvelope =
                serde_json::from_str(content).map_err(|_| schema_error())?;
            let plan = match envelope {
                PlanEnvelope::Decision(decision) => {
                    if decision.schema_version != SCHEMA_VERSION {
                        return Err(schema_error());
                    }
                    request
                        .pack_context
                        .plan(&request.task, &decision.recommended_treatment_id)
                }
                PlanEnvelope::Legacy(legacy) => {
                    if legacy.schema_version != SCHEMA_VERSION {
                        return Err(schema_error());
                    }
                    request
                        .pack_context
                        .ground_legacy(&request.task, &legacy.treatment_plan)
                }
            }
            .map_err(|_| AgentError {
                kind: AgentErrorKind::InvalidPlan,
                status: None,
            })?;
            Ok(AgentOutput {
                value: plan,
                provider: "digitalocean",
                model: DIGITALOCEAN_PLANNER_MODEL,
                deployment_version: self.planner_version.clone(),
                fallback: None,
            })
        })
    }

    fn generate<'a>(&'a self, request: GenerateRequest) -> AgentFuture<'a, CandidatePatch> {
        DeterministicWorkspaceAgent.generate(request)
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
    model: String,
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
#[serde(untagged)]
enum PlanEnvelope {
    Decision(PlanDecisionEnvelope),
    Legacy(LegacyPlanEnvelope),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanDecisionEnvelope {
    schema_version: u8,
    recommended_treatment_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPlanEnvelope {
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
    pack_context: &'a PackPlanningContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_treatment_id: Option<&'a str>,
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
            let planner_version = std::env::var("DIGITALOCEAN_PLANNER_VERSION")
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
                access_key,
                planner_version,
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

fn bounded_problem(task: &str) -> String {
    let normalized = task.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut end = normalized.len().min(500);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].to_string()
}

fn deterministic_patch(request: &GenerateRequest) -> Result<CandidatePatch> {
    request
        .accepted_files
        .get("web/src/lib/treatment.json")
        .context("accepted treatment configuration is missing")?;
    request
        .selected_treatment
        .refinement
        .validate()
        .map_err(anyhow::Error::msg)?;
    if request.features.is_empty() || request.features.len() > 64 {
        bail!("treatment feature snapshot must contain 1 to 64 items");
    }
    for feature in &request.features {
        if feature.trim().is_empty() || feature.len() > 300 || feature.chars().any(char::is_control)
        {
            bail!("treatment feature snapshot contains invalid text");
        }
    }
    let content = serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": 1,
        "treatment": &request.selected_treatment.treatment,
        "refinement": &request.selected_treatment.refinement,
        "features": &request.features,
        "planner": &request.selected_treatment.planner,
        "materializer": DETERMINISTIC_MATERIALIZER,
    }))?;
    Ok(CandidatePatch {
        summary: format!(
            "Apply {} as a {} Svelte treatment",
            request.selected_treatment.treatment.label,
            match request.selected_treatment.refinement.presentation {
                crate::workspace::PresentationMode::TaskFirst => "task-first",
                crate::workspace::PresentationMode::ContextFirst => "context-first",
            }
        ),
        files: vec![CandidateFile {
            path: "web/src/lib/treatment.json".into(),
            content,
            reason: format!(
                "Rust materialized the complete selected treatment for pack {} without changing server authority",
                request.pack
            ),
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

    fn planning_context() -> PackPlanningContext {
        let pack = crate::packs::builtin_packs()
            .into_iter()
            .find(|pack| pack.id == "post-op-monitor")
            .unwrap();
        PackPlanningContext::from_pack(&pack)
    }

    fn selected(treatment_id: &str) -> TreatmentSelection {
        let plan = planning_context().plan("queue", treatment_id).unwrap();
        TreatmentSelection {
            treatment: plan
                .treatments
                .iter()
                .find(|treatment| treatment.id == treatment_id)
                .unwrap()
                .clone(),
            refinement: crate::workspace::ClinicianRefinement::default(),
            plan_digest: crate::workspace::treatment_plan_digest(&plan),
            planner: crate::workspace::AgentProvenance {
                provider: "digitalocean".into(),
                model: DIGITALOCEAN_PLANNER_MODEL.into(),
                deployment_version: Some("planner-test-v1".into()),
                fallback_reason: None,
            },
            selected_by: "dr-test".into(),
            selected_at: 1,
        }
    }

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
            "key".into(),
            None,
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
        let plan = r#"{"schema_version":1,"recommended_treatment_id":"event-timeline"}"#;
        let body = serde_json::json!({
            "id": "completion-1",
            "model": "gemma-4-31B-it",
            "choices": [{"message": {"role": "assistant", "content": plan}}]
        })
        .to_string();
        let (endpoint, request) = serve_once(body);
        let agent = DigitalOceanWorkspaceAgent::new(
            &endpoint,
            "private-key".into(),
            Some("planner-test-v1".into()),
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
                pack_context: planning_context(),
            })
            .await
            .unwrap();
        assert_eq!(output.provider, "digitalocean");
        assert_eq!(output.model, "gemma-4-31B-it");
        assert_eq!(
            output.deployment_version.as_deref(),
            Some("planner-test-v1")
        );
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
        assert_eq!(
            inner["pack_context"]["treatment_recipes"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(output.value.recommended_treatment_id, "event-timeline");
        assert!(raw.starts_with("POST /api/v1/chat/completions "));
    }

    #[tokio::test]
    async fn generation_is_always_local_and_deterministic() {
        let agent = DigitalOceanWorkspaceAgent::new(
            "http://127.0.0.1:9",
            "private-key".into(),
            Some("planner-test-v1".into()),
            Duration::from_secs(1),
            DEFAULT_MAX_RESPONSE_BYTES,
        )
        .unwrap();
        let output = agent
            .generate(GenerateRequest {
                thread_id: "app-1".into(),
                pack: "intake".into(),
                workspace_summary: "checkpoint=0".into(),
                features: vec!["Review synthetic intake".into()],
                selected_treatment: selected("event-timeline"),
                accepted_files: BTreeMap::from([(
                    "web/src/lib/treatment.json".into(),
                    "{}".into(),
                )]),
            })
            .await
            .unwrap();
        assert_eq!(output.provider, "rust");
        assert_eq!(output.model, DETERMINISTIC_MATERIALIZER);
        assert!(output.deployment_version.is_none());
        assert!(output.value.files[0].content.contains("Event timeline"));
        assert!(output.value.files[0].content.contains("event-timeline"));
    }

    #[test]
    fn materialization_is_distinct_deterministic_and_keeps_prose_in_json() {
        let accepted_files = BTreeMap::from([
            ("web/src/lib/treatment.json".into(), "{}".into()),
            (
                "server/src/main.rs".into(),
                "const PAIN_ESCALATION_THRESHOLD: u8 = 7;".into(),
            ),
        ]);
        let mut hostile = selected("guided-worklist");
        hostile.refinement = crate::workspace::ClinicianRefinement {
            presentation: crate::workspace::PresentationMode::ContextFirst,
            emphasis: Some(
                "</script><script>globalThis.pwned=1</script> ${value} `quoted` & safe".into(),
            ),
        };
        let first = GenerateRequest {
            thread_id: "app-1".into(),
            pack: "post-op-monitor".into(),
            workspace_summary: "checkpoint=0".into(),
            features: vec![
                "Review synthetic check-ins".into(),
                "Escalate synthetic pain signals".into(),
            ],
            selected_treatment: hostile.clone(),
            accepted_files: accepted_files.clone(),
        };
        let first_patch = deterministic_patch(&first).unwrap();
        let repeat_patch = deterministic_patch(&first).unwrap();
        assert_eq!(first_patch, repeat_patch);
        assert_eq!(first_patch.files.len(), 1);
        assert_eq!(first_patch.files[0].path, "web/src/lib/treatment.json");
        let config: serde_json::Value =
            serde_json::from_str(&first_patch.files[0].content).unwrap();
        assert_eq!(
            config["refinement"]["emphasis"],
            hostile.refinement.emphasis.unwrap()
        );
        assert_eq!(config["materializer"], DETERMINISTIC_MATERIALIZER);
        assert_eq!(
            config["features"],
            serde_json::json!(first.features.clone())
        );

        let second = GenerateRequest {
            selected_treatment: selected("event-timeline"),
            ..first
        };
        let second_patch = deterministic_patch(&second).unwrap();
        assert_ne!(first_patch.files[0].content, second_patch.files[0].content);
        assert!(second_patch.files[0].content.contains("Event timeline"));
        assert!(!second_patch.files[0].content.contains("Guided worklist"));
        assert_eq!(
            accepted_files["server/src/main.rs"],
            "const PAIN_ESCALATION_THRESHOLD: u8 = 7;"
        );
    }

    #[test]
    fn deterministic_fallback_normalizes_and_bounds_the_problem() {
        let task = format!("First line\n\n{}", "é".repeat(400));
        let plan = planning_context().plan(&task, "guided-worklist").unwrap();
        assert!(plan.validate().is_ok());
        assert!(!plan.problem.contains('\n'));
        assert!(plan.problem.len() <= 500);
        assert!(plan.problem.starts_with("First line "));
    }

    #[tokio::test]
    async fn strict_envelope_rejects_unknown_fields() {
        let invalid_plan = r#"{"schema_version":1,"recommended_treatment_id":"guided-worklist","unexpected":true}"#;
        let body = serde_json::json!({
            "model": "gemma-4-31B-it",
            "choices": [{"message": {"content": invalid_plan}}]
        })
        .to_string();
        let (endpoint, _) = serve_once(body);
        let agent = DigitalOceanWorkspaceAgent::new(
            &endpoint,
            "private-key".into(),
            None,
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
                pack_context: planning_context(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.kind, AgentErrorKind::Schema);
        assert!(!error.to_string().contains("unexpected"));
    }

    #[tokio::test]
    async fn gemma_plan_rejects_executable_treatment_id() {
        let unsafe_id = "x');globalThis.__planner_xss=1;//";
        let plan = serde_json::json!({
            "schema_version": 1,
            "recommended_treatment_id": unsafe_id
        })
        .to_string();
        let body = serde_json::json!({
            "model": "gemma-4-31B-it",
            "choices": [{"message": {"content": plan}}]
        })
        .to_string();
        let (endpoint, _) = serve_once(body);
        let agent = DigitalOceanWorkspaceAgent::new(
            &endpoint,
            "private-key".into(),
            Some("planner-test-v1".into()),
            Duration::from_secs(5),
            DEFAULT_MAX_RESPONSE_BYTES,
        )
        .unwrap();
        let error = agent
            .plan(PlanRequest {
                thread_id: "app-1".into(),
                task: "queue".into(),
                pack: "post-op-monitor".into(),
                workspace_summary: "checkpoint=0".into(),
                pack_context: planning_context(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.kind, AgentErrorKind::InvalidPlan);
    }

    #[tokio::test]
    async fn invalid_gemma_plan_falls_back_honestly() {
        let unsafe_id = "x');globalThis.__planner_xss=1;//";
        let plan = serde_json::json!({
            "schema_version": 1,
            "recommended_treatment_id": unsafe_id
        })
        .to_string();
        let body = serde_json::json!({
            "model": "gemma-4-31B-it",
            "choices": [{"message": {"content": plan}}]
        })
        .to_string();
        let (endpoint, _) = serve_once(body);
        let primary = Arc::new(
            DigitalOceanWorkspaceAgent::new(
                &endpoint,
                "private-key".into(),
                Some("planner-test-v1".into()),
                Duration::from_secs(5),
                DEFAULT_MAX_RESPONSE_BYTES,
            )
            .unwrap(),
        );
        let agent = FallbackWorkspaceAgent::new(primary, Arc::new(DeterministicWorkspaceAgent));
        let output = agent
            .plan(PlanRequest {
                thread_id: "app-1".into(),
                task: "queue".into(),
                pack: "post-op-monitor".into(),
                workspace_summary: "checkpoint=0".into(),
                pack_context: planning_context(),
            })
            .await
            .unwrap();
        assert_eq!(output.provider, "deterministic");
        assert_eq!(output.model, DETERMINISTIC_MODEL);
        assert_eq!(
            output.fallback.as_ref().map(|error| error.kind),
            Some(AgentErrorKind::InvalidPlan)
        );
        output.value.validate().unwrap();
    }

    #[tokio::test]
    async fn generate_is_deterministic_when_no_worker_endpoint_is_configured() {
        let agent = DigitalOceanWorkspaceAgent::new(
            "http://127.0.0.1:9",
            "private-key".into(),
            None,
            Duration::from_secs(1),
            DEFAULT_MAX_RESPONSE_BYTES,
        )
        .unwrap();
        let output = agent
            .generate(GenerateRequest {
                thread_id: "app-1".into(),
                pack: "intake".into(),
                workspace_summary: "checkpoint=0".into(),
                features: vec!["Review synthetic intake".into()],
                selected_treatment: selected("guided-worklist"),
                accepted_files: BTreeMap::from([(
                    "web/src/lib/treatment.json".into(),
                    "{}".into(),
                )]),
            })
            .await
            .unwrap();
        assert_eq!(output.provider, "rust");
        assert_eq!(output.model, DETERMINISTIC_MATERIALIZER);
        assert!(output.fallback.is_none());
        assert!(output.value.files[0].content.contains("Guided worklist"));
    }
}
