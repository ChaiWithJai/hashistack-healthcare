//! Gate engine: the promotion checklist as code — the product.
//!
//! Gates are plugins behind one small trait, the way Vault mounts secret
//! engines and Nomad mounts task drivers. The platform ships a built-in set;
//! a hospital's own gates (IRB review, model risk) register alongside them.
//! The engine evaluates; it never deploys and never edits the app.
//!
//! Evidence over claims (#3): gates that can inspect artifacts implement the
//! optional [`Evidence`] capability — discovered per gate like a Nomad
//! optional plugin interface — and derive their verdict from the pack's
//! embedded scaffold source instead of the app record's self-reported
//! control set. Every result says which basis produced it, and a labeled
//! placeholder in the source yields a distinct `stubbed` verdict that is
//! never rendered as a pass (decision 0003).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use crate::packs::PackSourceFile;
use crate::state::{AppRecord, DataSource};

/// What a verdict rests on. `control` = the app record's self-reported
/// wired-control set — a claim. `evidence` = static analysis over the pack's
/// embedded scaffold artifacts — inspected, not asserted. The report carries
/// this per gate so it is honest about what each verdict is worth.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Basis {
    Control,
    Evidence,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum GateStatus {
    Pass,
    /// Evidence found the control's plumbing, but the mechanism is a labeled
    /// placeholder (e.g. the scaffold's encryption stub). A stub satisfies a
    /// Phase 0 sandbox gate — the substrate itself is labeled simulation —
    /// but it is never rendered as `pass`, so no report can claim (say)
    /// encryption that never happened. Decision 0003 records the shape.
    Stubbed {
        reason: String,
    },
    Fail {
        reason: String,
        fixable: bool,
    },
}

impl GateStatus {
    /// Satisfied for promotion: passing, or a labeled stub. Only a `Fail`
    /// blocks the deploy — but only a `Pass` is ever *called* a pass.
    pub fn satisfied(&self) -> bool {
        !matches!(self, GateStatus::Fail { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateResult {
    pub id: String,
    pub title: String,
    /// What this verdict rests on (control claim vs inspected evidence).
    pub basis: Basis,
    /// Dual-register vocabulary (P1): the HIPAA citation behind the plain-
    /// language title. `None` for pack-defined clinical gates with no direct
    /// citation. UI text is unchanged; exports render both registers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
    #[serde(flatten)]
    pub outcome: GateStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateReport {
    pub app_id: String,
    pub app_version: u32,
    pub results: Vec<GateResult>,
    /// Strictly `Pass` verdicts. Stubbed verdicts are counted apart so this
    /// number can never quietly absorb a placeholder.
    pub passed: usize,
    #[serde(default)]
    pub stubbed: usize,
    pub total: usize,
    /// No failures (every gate passed or is a labeled stub) and at least one
    /// gate ran. Green unlocks promotion; the stub count stays visible.
    pub green: bool,
}

impl GateReport {
    pub fn summary(&self) -> String {
        if self.stubbed == 0 {
            format!("{}/{}", self.passed, self.total)
        } else {
            format!("{}/{} ({} stubbed)", self.passed, self.total, self.stubbed)
        }
    }

    /// Gates that block promotion — failures only. Stubbed verdicts are
    /// surfaced by [`GateReport::stubbed`] and the per-result status, not
    /// here: they gate honesty, not the deploy button.
    pub fn failing(&self) -> Vec<&GateResult> {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, GateStatus::Fail { .. }))
            .collect()
    }
}

/// One compliance check. `evaluate` must be pure and cheap over the app
/// record: same app in, same verdict out, so a gate report is reproducible
/// evidence — this is the side-effect-free *validate* phase of Packer's
/// prepare/run split (steering §3), safe to dry-run on every keystroke.
pub trait Gate: Send + Sync {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn evaluate(&self, app: &AppRecord) -> GateStatus;
    /// The optional evidence capability, discovered the way Nomad probes a
    /// plugin for an optional interface. Default: not supported — the engine
    /// falls back to the control basis wherever the capability (or the
    /// pack's scaffold) is absent.
    fn as_evidence(&self) -> Option<&dyn Evidence> {
        None
    }
}

/// The *execute* half of the Packer split: derive the verdict from artifacts.
/// Still deterministic and side-effect-free — Phase 0 evidence is static
/// analysis over the pack's compile-time-embedded scaffold sources, never
/// execution — so the whole gate plan remains dry-runnable.
pub trait Evidence {
    fn inspect(&self, app: &AppRecord, ctx: &EvidenceContext) -> GateStatus;
}

/// The artifacts an evidence-capable gate may inspect: the pack's embedded
/// scaffold file map and its signed network allowlist. Built from the
/// compile-time pack tables for real packs; tests construct it directly to
/// point the same gates at adversarial fixtures (tests/evidence_contract.rs).
pub struct EvidenceContext {
    /// (pack-relative path, content) — the embedded scaffold sources.
    pub files: &'static [PackSourceFile],
    /// Hosts declared in the pack's policies/network-allowlist.hcl.
    pub allowlist: Vec<String>,
}

impl EvidenceContext {
    /// The evidence context for a pack, if it ships a runnable scaffold.
    /// Packs without one get `None` and keep control-basis verdicts.
    pub fn for_pack(pack_id: &str) -> Option<Self> {
        let files = crate::packs::scaffold_sources(pack_id)?;
        Some(Self {
            files,
            allowlist: crate::packs::network_allowlist(pack_id).unwrap_or_default(),
        })
    }

    fn rust_sources(&self) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
        self.files
            .iter()
            .filter(|(path, _)| path.ends_with(".rs"))
            .copied()
    }
}

/// Dual-register vocabulary (P1, review-log round 1): the UI keeps clinical
/// plain language; exports and the report JSON additionally carry the HIPAA
/// citation each gate implements. Pack-defined clinical gates with no direct
/// citation are honestly absent rather than stretched to fit.
const HIPAA_CITATIONS: &[(&str, &str)] = &[
    (
        "phi-encryption",
        "45 CFR §164.312(a)(2)(iv), §164.312(e)(2)(ii)",
    ),
    ("audit-log", "45 CFR §164.312(b)"),
    ("auto-logoff", "45 CFR §164.312(a)(2)(iii)"),
    ("access-roles", "45 CFR §164.312(a)(1)"),
    ("ai-allowlist", "45 CFR §164.308(b)(1), §164.312(e)(1)"),
    ("dependency-scan", "45 CFR §164.308(a)(5)(ii)(B)"),
    ("synthetic-only", "45 CFR §164.514(b)"),
    ("human-review", "45 CFR §164.308(a)(8)"),
    // escalation-path: clinical safety semantics defined by the pack
    // (packs/*/gates/README.md) — no direct HIPAA citation, by design.
];

/// The HIPAA citation for a gate id, when one exists.
pub fn hipaa_citation(id: &str) -> Option<&'static str> {
    HIPAA_CITATIONS
        .iter()
        .find(|(gate_id, _)| *gate_id == id)
        .map(|(_, citation)| *citation)
}

/// The control-basis check most HIPAA technical safeguards reduce to: is the
/// control wired on the app record?
fn control_check(app: &AppRecord, id: &str, missing: &str, fixable: bool) -> GateStatus {
    if app.controls.contains(id) {
        GateStatus::Pass
    } else {
        GateStatus::Fail {
            reason: missing.to_string(),
            fixable,
        }
    }
}

/// A gate satisfied by a wired control on the app record.
///
/// TODO(#3), narrowed: controls are still self-reported by the scaffold and
/// agent, so a control-basis verdict is a claim, not evidence. The gates
/// below carry an [`Evidence`] implementation over packs that ship a real
/// scaffold (post-op-monitor today); still pending under this ticket:
/// a real dependency scanner (cargo-audit/osv-scanner) behind an Evidence
/// impl for dependency-scan, egress observed from the sandbox allocation
/// (not just source literals), and evidence coverage for the remaining four
/// packs as #5 converts them to runnable scaffolds.
struct ControlGate {
    id: &'static str,
    title: &'static str,
    missing: &'static str,
    fixable: bool,
}

impl Gate for ControlGate {
    fn id(&self) -> &'static str {
        self.id
    }
    fn title(&self) -> &'static str {
        self.title
    }
    fn evaluate(&self, app: &AppRecord) -> GateStatus {
        control_check(app, self.id, self.missing, self.fixable)
    }
}

// ---------- audit-log: evidence = a textual walk of the router builder ----

/// Every data-touching route must pass through the audit middleware.
///
/// Evidence basis: a textual walk of the scaffold's axum `Router` builder.
/// In axum, `.layer(...)` wraps only the routes registered *before* it, so
/// the rule is: every `.route("…", …)` registration must appear before the
/// last audit `.layer(…)` in its file. Honest limits of textual analysis:
/// it sees one builder chain per file, cannot follow routers built across
/// functions, `merge`/`nest` composition, or conditional layering — a
/// scaffold using those needs a real AST walker (still #3). Within those
/// limits it cannot false-pass: a route added after (or without) the audit
/// layer fails, named.
struct AuditLogGate;

impl Gate for AuditLogGate {
    fn id(&self) -> &'static str {
        "audit-log"
    }
    fn title(&self) -> &'static str {
        "audit log on every data access"
    }
    fn evaluate(&self, app: &AppRecord) -> GateStatus {
        control_check(
            app,
            self.id(),
            "a data-touching route is missing hipaa-core audit middleware",
            false,
        )
    }
    fn as_evidence(&self) -> Option<&dyn Evidence> {
        Some(self)
    }
}

impl Evidence for AuditLogGate {
    fn inspect(&self, _app: &AppRecord, ctx: &EvidenceContext) -> GateStatus {
        let mut any_route = false;
        let mut unaudited: Vec<String> = Vec::new();
        for (file, src) in ctx.rust_sources() {
            let routes = route_registrations(src);
            if routes.is_empty() {
                continue;
            }
            any_route = true;
            // A route is audited iff an audit layer is registered after it
            // (axum: later layers wrap earlier routes).
            let last_audit = audit_layer_positions(src).into_iter().max();
            for (pos, path) in routes {
                match last_audit {
                    Some(layer_pos) if pos < layer_pos => {}
                    _ => unaudited.push(format!("{path} ({file})")),
                }
            }
        }
        if !any_route {
            return GateStatus::Fail {
                reason: "no `.route(…)` registrations found in the scaffold source — \
                         the textual route walker has nothing to audit"
                    .to_string(),
                fixable: false,
            };
        }
        if unaudited.is_empty() {
            GateStatus::Pass
        } else {
            GateStatus::Fail {
                reason: format!(
                    "routes registered outside the audit middleware layer: {}",
                    unaudited.join(", ")
                ),
                fixable: false,
            }
        }
    }
}

/// Byte offsets and path literals of `.route("…"` registrations.
fn route_registrations(src: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (pos, _) in src.match_indices(".route(") {
        let rest = &src[pos + ".route(".len()..];
        if let Some(stripped) = rest.strip_prefix('"') {
            if let Some(end) = stripped.find('"') {
                found.push((pos, stripped[..end].to_string()));
            }
        }
    }
    found
}

/// Byte offsets of `.layer(…)` calls whose argument text (same line — the
/// builder chain writes one layer per line) mentions audit.
fn audit_layer_positions(src: &str) -> Vec<usize> {
    src.match_indices(".layer(")
        .filter(|(pos, _)| {
            let line_end = src[*pos..].find('\n').map_or(src.len(), |n| pos + n);
            src[*pos..line_end].contains("audit")
        })
        .map(|(pos, _)| pos)
        .collect()
}

// ---------- phi-encryption: evidence = phi markers vs encryption sites ----

/// Fields marked PHI must flow through field-level encryption.
///
/// Marker convention (defined here, used by scaffold authors):
/// - `// phi:` on a struct field declares it part of the PHI inventory.
/// - `// phi-encryption: <disposition>` inside the same struct declares how
///   those fields are protected: `vault-transit` (real — the file must then
///   contain a `hipaa_core::encrypt_field(` call site) or `stub` (a labeled
///   placeholder).
///
/// Verdict shape — the heart of #3, no false passes:
/// - a PHI field in a struct with no disposition, an unknown disposition, or
///   a `vault-transit` claim with no call site in the file → **Fail**, named;
/// - any field resting on a `stub` disposition → **Stubbed**: the plumbing
///   is verified and honestly labeled, but the report never says "pass" for
///   a cipher that does not exist;
/// - all fields on verified `vault-transit` → Pass;
/// - no `// phi:` markers anywhere → **Fail**: a missing PHI inventory is
///   not evidence of encryption, it is the absence of evidence.
///
/// Limits, honestly: the walker is textual and struct-scoped — it trusts
/// marker placement and cannot trace dataflow. What it can never do is
/// upgrade a stub to a pass.
struct PhiEncryptionGate;

impl Gate for PhiEncryptionGate {
    fn id(&self) -> &'static str {
        "phi-encryption"
    }
    fn title(&self) -> &'static str {
        "encryption on all patient fields"
    }
    fn evaluate(&self, app: &AppRecord) -> GateStatus {
        control_check(
            app,
            self.id(),
            "one or more PHI fields lack hipaa-core field-level encryption",
            false,
        )
    }
    fn as_evidence(&self) -> Option<&dyn Evidence> {
        Some(self)
    }
}

impl Evidence for PhiEncryptionGate {
    fn inspect(&self, _app: &AppRecord, ctx: &EvidenceContext) -> GateStatus {
        let mut stubbed: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        let mut verified = 0usize;
        let mut any_marker = false;

        for (file, src) in ctx.rust_sources() {
            for block in struct_blocks(src) {
                let fields = phi_fields(block.body);
                if fields.is_empty() {
                    continue;
                }
                any_marker = true;
                let label = format!("{} [{}] ({file})", block.name, fields.join(", "));
                let disposition = block
                    .body
                    .lines()
                    .find_map(|line| line.split("// phi-encryption:").nth(1))
                    .map(str::trim);
                match disposition {
                    None => failed.push(format!(
                        "{label}: PHI fields with no declared encryption path"
                    )),
                    Some(d) if d.starts_with("vault-transit") => {
                        if src.contains("hipaa_core::encrypt_field(") {
                            verified += fields.len();
                        } else {
                            failed.push(format!(
                                "{label}: declares vault-transit but the file has no hipaa_core::encrypt_field( call site"
                            ));
                        }
                    }
                    Some(d) if d.starts_with("stub") => stubbed.push(label),
                    Some(d) => failed.push(format!("{label}: unknown disposition {d:?}")),
                }
            }
        }

        if !any_marker {
            return GateStatus::Fail {
                reason: "no `// phi:` field markers found in the scaffold source — the PHI \
                         inventory is missing, so encryption cannot be evidenced"
                    .to_string(),
                fixable: false,
            };
        }
        if !failed.is_empty() {
            return GateStatus::Fail {
                reason: failed.join("; "),
                fixable: false,
            };
        }
        if !stubbed.is_empty() {
            return GateStatus::Stubbed {
                reason: format!(
                    "PHI fields rest on a labeled encryption stub — plumbing verified, cipher \
                     absent (hipaa-core encryptField via Vault transit is the scaffold's \
                     labeled TODO): {}",
                    stubbed.join("; ")
                ),
            };
        }
        debug_assert!(verified > 0);
        GateStatus::Pass
    }
}

struct StructBlock<'a> {
    name: &'a str,
    body: &'a str,
}

/// Brace-matched `struct Name { … }` bodies in a source file. Tuple and unit
/// structs carry no named fields and are skipped.
fn struct_blocks(src: &str) -> Vec<StructBlock<'_>> {
    let mut blocks = Vec::new();
    for (pos, _) in src.match_indices("struct ") {
        let after = &src[pos + "struct ".len()..];
        let name_end = after
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        let name = &after[..name_end];
        let rest = &after[name_end..];
        // Only brace-bodied structs; `;` before `{` means tuple/unit.
        let Some(open) = rest.find('{') else { continue };
        if rest[..open].contains(';') || rest[..open].contains('(') {
            continue;
        }
        let body_start = open + 1;
        let mut depth = 1usize;
        let mut end = None;
        for (i, c) in rest[body_start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(body_start + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            blocks.push(StructBlock {
                name,
                body: &rest[body_start..end],
            });
        }
    }
    blocks
}

/// Field names on lines carrying the `// phi:` marker.
fn phi_fields(body: &str) -> Vec<String> {
    body.lines()
        .filter(|line| line.contains("// phi:"))
        .filter_map(|line| {
            let decl = line.split("//").next()?.trim();
            let name = decl.strip_prefix("pub ").unwrap_or(decl);
            let name = name.split(':').next()?.trim();
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

// ---------- ai-allowlist ------------------------------------------------

/// Third-party calls must resolve against the BAA'd allowlist. An un-BAA'd
/// AI endpoint is the single most common way a vibe-coded tool leaks PHI.
///
/// Evidence basis: every URL/host literal in the scaffold sources, plus the
/// app record's declared external calls, checked against the pack's signed
/// policies/network-allowlist.hcl — so widening the app's reach is a signed
/// pack revision, not an app edit. Loopback/bind hosts are exempt (local
/// listeners, not egress). Limits: literals only — dynamically composed
/// URLs need the observed-egress evidence still queued under #3.
struct AiAllowlistGate;

/// Control-basis fallback for packs without a scaffold (their manifests
/// have no allowlist policy file to check literals against).
const ENDPOINT_ALLOWLIST: &[&str] = &[
    "vault.internal",
    "postgres.internal",
    "api.anthropic.com", // platform LLM key, scoped per environment, under BAA
];

const LOCAL_HOSTS: &[&str] = &["127.0.0.1", "0.0.0.0", "localhost"];

impl Gate for AiAllowlistGate {
    fn id(&self) -> &'static str {
        "ai-allowlist"
    }
    fn title(&self) -> &'static str {
        "no un-approved AI calls"
    }
    fn evaluate(&self, app: &AppRecord) -> GateStatus {
        let rogue: Vec<&str> = app
            .external_calls
            .iter()
            .map(String::as_str)
            .filter(|c| !ENDPOINT_ALLOWLIST.contains(c))
            .collect();
        if rogue.is_empty() {
            GateStatus::Pass
        } else {
            GateStatus::Fail {
                reason: format!(
                    "calls endpoints outside the BAA allowlist: {}",
                    rogue.join(", ")
                ),
                fixable: false,
            }
        }
    }
    fn as_evidence(&self) -> Option<&dyn Evidence> {
        Some(self)
    }
}

impl Evidence for AiAllowlistGate {
    fn inspect(&self, app: &AppRecord, ctx: &EvidenceContext) -> GateStatus {
        let mut rogue = BTreeSet::new();
        for (file, src) in ctx.files {
            for host in host_literals(src) {
                if LOCAL_HOSTS.contains(&host.as_str()) {
                    continue;
                }
                if !ctx.allowlist.iter().any(|a| a == &host) {
                    rogue.insert(format!("{host} ({file})"));
                }
            }
        }
        for call in &app.external_calls {
            if !ctx.allowlist.iter().any(|a| a == call) {
                rogue.insert(format!("{call} (declared external call)"));
            }
        }
        if rogue.is_empty() {
            GateStatus::Pass
        } else {
            GateStatus::Fail {
                reason: format!(
                    "hosts outside the pack's signed network allowlist \
                     (policies/network-allowlist.hcl): {}",
                    rogue.into_iter().collect::<Vec<_>>().join(", ")
                ),
                fixable: false,
            }
        }
    }
}

/// Host parts of `http://` / `https://` literals in a source text. Template
/// interpolations (`http://{bind}`) yield an empty host and are skipped.
fn host_literals(src: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for (pos, _) in src.match_indices("://") {
        let scheme_ok = src[..pos].ends_with("http") || src[..pos].ends_with("https");
        if !scheme_ok {
            continue;
        }
        let rest = &src[pos + 3..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .unwrap_or(rest.len());
        let host = &rest[..end];
        if !host.is_empty() {
            hosts.push(host.to_string());
        }
    }
    hosts
}

// ---------- synthetic-only ----------------------------------------------

/// The sandbox must only ever have seen synthetic data. This is evaluated,
/// not assumed, so the gate report can attest to it.
///
/// Evidence basis adds a source assertion to the runtime check: the scaffold
/// must carry the boot guard that refuses any dataset without the SYNTHETIC
/// notice — so the guarantee holds in the ejected app too, not just under
/// this control plane.
struct SyntheticOnlyGate;

impl Gate for SyntheticOnlyGate {
    fn id(&self) -> &'static str {
        "synthetic-only"
    }
    fn title(&self) -> &'static str {
        "sandbox saw synthetic data only"
    }
    fn evaluate(&self, app: &AppRecord) -> GateStatus {
        match &app.data_source {
            DataSource::Synthetic(_) => GateStatus::Pass,
            DataSource::Tenant(db) => GateStatus::Fail {
                reason: format!("sandbox is wired to tenant data source {db}"),
                fixable: false,
            },
        }
    }
    fn as_evidence(&self) -> Option<&dyn Evidence> {
        Some(self)
    }
}

impl Evidence for SyntheticOnlyGate {
    fn inspect(&self, app: &AppRecord, ctx: &EvidenceContext) -> GateStatus {
        let guarded = ctx
            .rust_sources()
            .any(|(_, src)| src.contains("SYNTHETIC") && src.contains("refusing to boot"));
        if !guarded {
            return GateStatus::Fail {
                reason: "scaffold source carries no SYNTHETIC-notice boot guard — it would \
                         accept an unmarked dataset"
                    .to_string(),
                fixable: false,
            };
        }
        // The runtime half still holds: the record must be synthetic-wired.
        self.evaluate(app)
    }
}

// ---------- registry ------------------------------------------------------

static REGISTRY: LazyLock<Vec<Box<dyn Gate>>> = LazyLock::new(|| {
    vec![
        Box::new(PhiEncryptionGate),
        Box::new(AuditLogGate),
        Box::new(AiAllowlistGate),
        Box::new(ControlGate {
            id: "dependency-scan",
            title: "dependency scan clean",
            missing: "dependency scan has unresolved findings",
            fixable: false,
        }),
        Box::new(ControlGate {
            id: "auto-logoff",
            title: "auto-logoff after idle",
            missing: "auto-logoff after idle — not wired",
            fixable: true,
        }),
        Box::new(SyntheticOnlyGate),
        Box::new(ControlGate {
            id: "access-roles",
            title: "access roles for staff",
            missing: "staff-facing surface has no role-based access control",
            fixable: true,
        }),
        Box::new(ControlGate {
            id: "escalation-path",
            title: "clinical escalation path",
            missing: "no escalation path for out-of-range or urgent findings",
            fixable: true,
        }),
        Box::new(ControlGate {
            id: "human-review",
            title: "platform review attached",
            missing: "compliance review not yet run — request co-sign review",
            fixable: false,
        }),
    ]
});

fn gate(id: &str) -> Option<&'static dyn Gate> {
    REGISTRY.iter().find(|g| g.id() == id).map(|g| g.as_ref())
}

pub fn known_gate(id: &str) -> bool {
    gate(id).is_some()
}

pub fn gate_fixable(id: &str) -> bool {
    // Whether "fix it for me" may wire this control directly.
    matches!(id, "auto-logoff" | "access-roles" | "escalation-path")
}

/// Run the preflight: evaluate exactly the gates the app's pack requires,
/// in the pack's declared order, against the pack's evidence context when
/// its scaffold ships one.
pub fn preflight(app: &AppRecord, required: &[String]) -> GateReport {
    preflight_with_context(app, required, EvidenceContext::for_pack(&app.pack).as_ref())
}

/// The engine with an explicit evidence context — the seam through which
/// the adversarial fixtures point the same gates at a broken scaffold.
pub fn preflight_with_context(
    app: &AppRecord,
    required: &[String],
    ctx: Option<&EvidenceContext>,
) -> GateReport {
    let results: Vec<GateResult> = required
        .iter()
        .map(|id| match gate(id) {
            Some(g) => {
                let (outcome, basis) = match (ctx, g.as_evidence()) {
                    (Some(ctx), Some(evidence)) => (evidence.inspect(app, ctx), Basis::Evidence),
                    _ => (g.evaluate(app), Basis::Control),
                };
                GateResult {
                    id: g.id().to_string(),
                    title: g.title().to_string(),
                    basis,
                    citation: hipaa_citation(g.id()).map(str::to_string),
                    outcome,
                }
            }
            None => GateResult {
                id: id.clone(),
                title: format!("unknown gate {id}"),
                basis: Basis::Control,
                citation: None,
                outcome: GateStatus::Fail {
                    reason: format!("pack requires gate {id:?} but no such gate is registered"),
                    fixable: false,
                },
            },
        })
        .collect();
    let passed = results
        .iter()
        .filter(|r| r.outcome == GateStatus::Pass)
        .count();
    let stubbed = results
        .iter()
        .filter(|r| matches!(r.outcome, GateStatus::Stubbed { .. }))
        .count();
    let total = results.len();
    GateReport {
        app_id: app.id.clone(),
        app_version: app.current_version,
        green: passed + stubbed == total && total > 0,
        passed,
        stubbed,
        total,
        results,
    }
}

/// The platform reviewer's attestation note (storyboard 1c's co-sign card):
/// a plain-language verdict derived from the same report the modal shows.
pub fn reviewer_note(report: &GateReport, tier: u8) -> String {
    let audience = if tier >= 3 {
        "patient-facing"
    } else {
        "practice-facing"
    };
    if report.green {
        let stub_note = if report.stubbed > 0 {
            format!(
                " {} check(s) rest on labeled stubs — see the report's basis and status fields.",
                report.stubbed
            )
        } else {
            String::new()
        };
        format!(
            "Meets release criteria for a {audience} tool ({} checks green).{stub_note} \
             Re-review required if messaging, new data fields, or external calls are added.",
            report.summary()
        )
    } else {
        let failing: Vec<String> = report.failing().iter().map(|r| r.title.clone()).collect();
        format!(
            "Not ready to release: {} of {} checks failing — {}.",
            failing.len(),
            report.total,
            failing.join("; ")
        )
    }
}

/// Machine-readable gate map for the compliance meter (storyboard 1b). The
/// meter answers "does this block promotion?" — a labeled stub does not —
/// while the full report keeps the honest per-gate status.
pub fn meter(report: &GateReport) -> BTreeMap<String, bool> {
    report
        .results
        .iter()
        .map(|r| (r.id.clone(), r.outcome.satisfied()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Stage;
    use std::collections::BTreeSet;

    fn post_op_app() -> AppRecord {
        let pack = crate::packs::builtin_packs()
            .into_iter()
            .find(|p| p.id == "post-op-monitor")
            .unwrap();
        AppRecord {
            id: "app-test".to_string(),
            name: "test".to_string(),
            prompt: "post-op tracker".to_string(),
            pack: pack.id.clone(),
            stage: Stage::Sandbox,
            data_source: DataSource::Synthetic(pack.synthetic_dataset.clone()),
            controls: pack.gates.iter().cloned().collect::<BTreeSet<_>>(),
            external_calls: vec!["api.anthropic.com".to_string()],
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

    fn required() -> Vec<String> {
        crate::packs::builtin_packs()
            .into_iter()
            .find(|p| p.id == "post-op-monitor")
            .unwrap()
            .gates
    }

    fn result<'a>(report: &'a GateReport, id: &str) -> &'a GateResult {
        report.results.iter().find(|r| r.id == id).unwrap()
    }

    #[test]
    fn post_op_verdicts_are_evidence_based_and_the_stub_never_passes() {
        let app = post_op_app();
        let report = preflight(&app, &required());

        // Evidence basis wherever the scaffold can be inspected.
        for id in [
            "phi-encryption",
            "audit-log",
            "ai-allowlist",
            "synthetic-only",
        ] {
            assert_eq!(result(&report, id).basis, Basis::Evidence, "{id}");
        }
        // Control basis where no evidence capability exists yet.
        for id in ["dependency-scan", "auto-logoff"] {
            assert_eq!(result(&report, id).basis, Basis::Control, "{id}");
        }

        // The heart of #3: the scaffold's encryption is a labeled stub, so
        // the verdict is Stubbed — visible, satisfied-for-promotion, and
        // never rendered as a pass.
        let phi = &result(&report, "phi-encryption").outcome;
        assert!(
            matches!(phi, GateStatus::Stubbed { reason } if reason.contains("labeled encryption stub")),
            "{phi:?}"
        );
        assert_ne!(*phi, GateStatus::Pass);
        assert!(phi.satisfied());

        assert_eq!(result(&report, "audit-log").outcome, GateStatus::Pass);
        assert_eq!(result(&report, "ai-allowlist").outcome, GateStatus::Pass);
        assert_eq!(result(&report, "synthetic-only").outcome, GateStatus::Pass);

        // Counters keep the stub out of `passed` and green stays honest.
        assert_eq!(report.passed, 5);
        assert_eq!(report.stubbed, 1);
        assert!(report.green);
        assert_eq!(report.summary(), "5/6 (1 stubbed)");
        assert!(report.failing().is_empty());
    }

    #[test]
    fn packs_without_a_scaffold_keep_control_basis_verdicts() {
        let mut app = post_op_app();
        app.pack = "hypertension-tracker".to_string();
        let report = preflight(&app, &required());
        assert!(report.results.iter().all(|r| r.basis == Basis::Control));
        assert_eq!(report.stubbed, 0);
        assert_eq!(report.summary(), "6/6");
        // Control basis still trusts the record — exactly the claim-shaped
        // verdict the evidence pass exists to replace.
        assert_eq!(result(&report, "phi-encryption").outcome, GateStatus::Pass);
    }

    #[test]
    fn evidence_overrides_a_self_reported_control() {
        // A record claiming phi-encryption cannot get a Pass past the
        // evidence: the scaffold's stub yields Stubbed regardless of the
        // controls set; dropping the control changes nothing either.
        let mut app = post_op_app();
        app.controls.remove("phi-encryption");
        let report = preflight(&app, &required());
        assert!(matches!(
            result(&report, "phi-encryption").outcome,
            GateStatus::Stubbed { .. }
        ));
        // audit-log likewise ignores the claim in both directions.
        app.controls.remove("audit-log");
        let report = preflight(&app, &required());
        assert_eq!(result(&report, "audit-log").outcome, GateStatus::Pass);
    }

    #[test]
    fn citations_ride_the_report_and_absent_ones_are_honest() {
        let app = post_op_app();
        let report = preflight(&app, &required());
        assert_eq!(
            result(&report, "audit-log").citation.as_deref(),
            Some("45 CFR §164.312(b)")
        );
        assert_eq!(
            result(&report, "auto-logoff").citation.as_deref(),
            Some("45 CFR §164.312(a)(2)(iii)")
        );
        assert_eq!(hipaa_citation("escalation-path"), None);
        // The JSON shape: citation present, basis tagged, stub distinct.
        let json = serde_json::to_value(&report).unwrap();
        let phi = json["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "phi-encryption")
            .unwrap();
        assert_eq!(phi["basis"], "evidence");
        assert_eq!(phi["status"], "stubbed");
        assert!(phi["citation"]
            .as_str()
            .unwrap()
            .contains("164.312(a)(2)(iv)"));
    }

    #[test]
    fn report_round_trips_through_serde_for_frozen_attestations() {
        // F3: the attestation stores the report verbatim; it must survive
        // serialization without losing basis, citation, or the stub status.
        let app = post_op_app();
        let report = preflight(&app, &required());
        let json = serde_json::to_string(&report).unwrap();
        let back: GateReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.summary(), report.summary());
        assert_eq!(back.results.len(), report.results.len());
        assert!(matches!(
            back.results
                .iter()
                .find(|r| r.id == "phi-encryption")
                .unwrap()
                .outcome,
            GateStatus::Stubbed { .. }
        ));
        assert_eq!(
            back.results
                .iter()
                .find(|r| r.id == "audit-log")
                .unwrap()
                .basis,
            Basis::Evidence
        );
    }

    #[test]
    fn meter_marks_stubs_satisfied_but_fails_stay_red() {
        let mut app = post_op_app();
        app.controls.remove("auto-logoff");
        let report = preflight(&app, &required());
        let meter = meter(&report);
        assert!(meter["phi-encryption"], "a labeled stub does not block");
        assert!(!meter["auto-logoff"]);
        assert!(!report.green);
        let note = reviewer_note(&report, 3);
        assert!(note.contains("1 of 6 checks failing"));
        // Green note discloses the stub count.
        app.controls.insert("auto-logoff".to_string());
        let note = reviewer_note(&preflight(&app, &required()), 3);
        assert!(note.contains("Meets release criteria"));
        assert!(note.contains("labeled stubs"));
    }
}
