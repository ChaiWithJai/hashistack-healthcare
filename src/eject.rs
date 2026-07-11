//! Ejection service: one verb — bundle. Turns a doctor's app record into a
//! self-contained repository they own outright: documentation generated from
//! *their* record (prompt, addenda, gate report, attestation, audit trail),
//! deploy manifests for four targets, and a pack.hcl derived from the app so
//! what they built becomes their own re-importable template. No hostage code,
//! no hostage docs (GOAL.md bars 5 and 6, issue #11).
//!
//! The bundle is a JSON file-map — zero archive dependencies; the unpack
//! one-liner in the response writes it to disk with stock python3.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::audit::AuditEvent;
use crate::deploy;
use crate::gates::{self, GateReport, GateStatus};
use crate::packs::PackManifest;
use crate::state::{AppRecord, DataSource, Stage};

/// Writes the file-map to disk from stdin. Stock python3, no dependencies —
/// pipe the export JSON through it inside the target directory.
pub const UNPACK_ONE_LINER: &str = r#"python3 -c 'import json,sys,pathlib; [(lambda q: (q.parent.mkdir(parents=True,exist_ok=True), q.write_text(c)))(pathlib.Path(p)) for p,c in json.load(sys.stdin)["files"].items()]'"#;

#[derive(Clone, Debug, Serialize)]
pub struct EjectionBundle {
    /// Relative path → file content. BTreeMap so the listing is stable.
    pub files: BTreeMap<String, String>,
    /// Copy-paste command that unpacks this bundle into the current directory.
    pub unpack: String,
}

/// Build the ejection bundle. Pure over its inputs: the same record and
/// audit slice always produce the same bundle, so an export is evidence.
pub fn bundle(app: &AppRecord, pack: &PackManifest, audit: &[&AuditEvent]) -> EjectionBundle {
    let report = preflight_report(app, pack);
    let mut files = BTreeMap::new();
    files.insert("README.md".to_string(), readme_md(app, pack));
    files.insert("docs/RUNBOOK.md".to_string(), runbook_md(app));
    files.insert(
        "docs/COMPLIANCE.md".to_string(),
        compliance_md(app, &report, audit),
    );
    files.insert("Dockerfile".to_string(), dockerfile(app));
    files.insert("render.yaml".to_string(), render_yaml(app));
    files.insert("fly.toml".to_string(), fly_toml(app));
    files.insert("config/deploy.yml".to_string(), kamal_deploy_yml(app));
    files.insert("nomad/job.nomad.hcl".to_string(), nomad_job(app));
    files.insert("pack.hcl".to_string(), derived_pack_hcl(app, pack));
    EjectionBundle {
        files,
        unpack: unpack_command(&app.id),
    }
}

/// Re-run the preflight for the compliance record. A live allocation reads
/// tenant data by design, and the synthetic-only gate attests the *sandbox
/// lineage* — so a released app is evaluated the way the gate engine saw it
/// at promotion, not against its (legitimately tenant-backed) live view.
fn preflight_report(app: &AppRecord, pack: &PackManifest) -> GateReport {
    if app.stage == Stage::Live {
        let mut release_view = app.clone();
        release_view.data_source = DataSource::Synthetic(pack.synthetic_dataset.clone());
        gates::preflight(&release_view, &pack.gates)
    } else {
        gates::preflight(app, &pack.gates)
    }
}

fn unpack_command(app_id: &str) -> String {
    format!("mkdir -p {app_id} && cd {app_id} && curl -s $PLATFORM/api/apps/{app_id}/export | {UNPACK_ONE_LINER}")
}

// ---------- README.md: the doctor's own story ----------

fn readme_md(app: &AppRecord, pack: &PackManifest) -> String {
    let mut md = String::new();
    md.push_str(&format!("# {}\n\n", app.name));
    md.push_str("Built on the clinician platform and ejected as an owned, self-contained\nrepository. It started as one sentence:\n\n");
    md.push_str(&format!("> {}\n\n", app.prompt));
    md.push_str(&format!(
        "Scaffolded from the **{}** pack (`{}`), HIPAA controls pre-wired: {}\n\n",
        pack.name, pack.id, pack.description
    ));

    md.push_str(&format!(
        "## What the app does today (v{})\n\n",
        app.current_version
    ));
    for feature in &app.features {
        md.push_str(&format!("- {feature}\n"));
    }

    md.push_str("\n## Changelog — the addenda record\n\nEvery conversational edit, logged like a chart addendum. This is the app's\nreal history, not release notes written after the fact.\n\n");
    for addendum in &app.addenda {
        md.push_str(&format!(
            "### v{} — {} ({})\n\n",
            addendum.version,
            addendum.instruction,
            utc(addendum.at)
        ));
        md.push_str(&format!("{}\n\n", addendum.reply));
        if let Some(feature) = &addendum.added_feature {
            md.push_str(&format!("- added feature: {feature}\n"));
        }
        if !addendum.wired_controls.is_empty() {
            md.push_str(&format!(
                "- wired controls: {}\n",
                addendum.wired_controls.join(", ")
            ));
        }
        md.push('\n');
    }

    md.push_str("## Owning it\n\n");
    md.push_str(&format!(
        "- [docs/RUNBOOK.md](docs/RUNBOOK.md) — run and deploy this bundle, no platform access needed.\n\
         - [docs/COMPLIANCE.md](docs/COMPLIANCE.md) — the gate report, attestation, and audit trail.\n\
         - [pack.hcl](pack.hcl) — this app as your own template (`{}-template`): re-import it,\n\
           share it with your practice, or submit it to the registry.\n",
        app.id
    ));
    md
}

// ---------- RUNBOOK.md: a stranger gets it running from this alone ----------

fn runbook_md(app: &AppRecord) -> String {
    let id = &app.id;
    format!(
        "# Runbook — {name}\n\n\
         This bundle is self-contained. Nothing here phones home to the platform.\n\n\
         ## Honest caveat: the app source is a scaffold placeholder\n\n\
         Until the platform's runnable-scaffold work (platform issue #5) lands, this\n\
         bundle does **not** include generated application source. The Dockerfile\n\
         builds a stub that serves `/health` on port 8080 so every deploy manifest\n\
         below is exercisable end-to-end today. The record, documentation, gate\n\
         report, and manifests are real; the runtime is the placeholder.\n\n\
         ## Unpack (if you received the raw export JSON)\n\n\
         ```bash\n\
         {unpack}\n\
         ```\n\n\
         ## Run locally\n\n\
         ```bash\n\
         docker build -t {id} .\n\
         docker run --rm -p 8080:8080 {id}\n\
         curl http://127.0.0.1:8080/health   # → ok\n\
         ```\n\n\
         ## Deploy targets\n\n\
         | target | manifest | command |\n\
         |---|---|---|\n\
         | Nomad | `nomad/job.nomad.hcl` | `nomad job run nomad/job.nomad.hcl` |\n\
         | Render | `render.yaml` | connect the repo; Render reads the blueprint |\n\
         | Fly.io | `fly.toml` | `fly launch --copy-config --now` |\n\
         | Kamal | `config/deploy.yml` | `kamal setup` (fill in your server + registry) |\n\n\
         The Nomad job is the platform's own rendered allocation spec; the Vault\n\
         `{{{{ with secret … }}}}` blocks resolve against your Vault at runtime.\n\
         Render/Fly/Kamal manifests build from the Dockerfile in this bundle.\n\n\
         ## Re-import as a template\n\n\
         `pack.hcl` at the bundle root is this app expressed in the platform's pack\n\
         schema — drop it into a platform's `packs/` directory (or submit it to the\n\
         registry) and \"{name}\" becomes a starting point instead of a one-off.\n",
        name = app.name,
        unpack = unpack_command(id),
        id = id,
    )
}

// ---------- COMPLIANCE.md: gate report + attestation + audit trail ----------

fn compliance_md(app: &AppRecord, report: &GateReport, audit: &[&AuditEvent]) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Compliance record — {}\n\n", app.name));

    match (&app.stage, &app.attestation) {
        (Stage::Live, Some(att)) => {
            md.push_str(&format!(
                "Status: **released** — live since {}, co-signed by {}.\n\n",
                utc(att.at),
                att.cosigner
            ));
        }
        _ => {
            md.push_str(
                "Status: **draft — not released.** This app is still in the sandbox: it\n\
                 has not been promoted, no clinician has co-signed it, and there is no\n\
                 attestation. The gate report below is a preflight snapshot, not a\n\
                 release record.\n\n",
            );
        }
    }

    md.push_str(&format!(
        "## Gate report (re-run at export, app v{})\n\n{} checks passed — {}\n\n",
        report.app_version,
        report.summary(),
        if report.green {
            "green"
        } else {
            "NOT green: promotion is locked until every check passes"
        }
    ));
    md.push_str("| gate | check | verdict |\n|---|---|---|\n");
    for result in &report.results {
        let verdict = match &result.outcome {
            GateStatus::Pass => "pass".to_string(),
            GateStatus::Fail { reason, fixable } => format!(
                "FAIL — {}{}",
                md_cell(reason),
                if *fixable { " (one-click fixable)" } else { "" }
            ),
        };
        md.push_str(&format!(
            "| `{}` | {} | {} |\n",
            result.id,
            md_cell(&result.title),
            verdict
        ));
    }

    md.push_str("\n## Attestation\n\n");
    match (&app.stage, &app.attestation) {
        (Stage::Live, Some(att)) => {
            md.push_str(&format!(
                "- co-signed by: **{}**\n- gate summary at release: **{}**\n",
                att.cosigner, att.gate_summary
            ));
            if let Some(note) = &att.reviewer_note {
                md.push_str(&format!("- platform reviewer note: {note}\n"));
            }
            md.push_str(&format!("- at: {}\n", utc(att.at)));
        }
        _ => {
            md.push_str(
                "None — omitted by design. An attestation exists only after a green gate\n\
                 report and a clinician co-signature at promotion.\n",
            );
        }
    }

    md.push_str("\n## Audit trail (append-only, as exported)\n\n");
    md.push_str("| seq | at | actor | action | detail |\n|---|---|---|---|---|\n");
    for event in audit {
        md.push_str(&format!(
            "| {} | {} | {} | `{}` | {} |\n",
            event.seq,
            utc(event.at),
            md_cell(&event.actor),
            event.action,
            md_cell(&event.detail)
        ));
    }
    md
}

// ---------- deploy manifests ----------

fn dockerfile(app: &AppRecord) -> String {
    format!(
        "# {} — placeholder runtime (see docs/RUNBOOK.md, \"Honest caveat\").\n\
         # The generated app source ships when the platform's runnable scaffolds\n\
         # (issue #5) land; until then this image serves /health on 8080 so every\n\
         # deploy manifest in this bundle works end-to-end today.\n\
         FROM python:3.12-alpine\n\
         WORKDIR /srv\n\
         RUN printf 'ok' > health\n\
         EXPOSE 8080\n\
         CMD [\"python3\", \"-m\", \"http.server\", \"8080\"]\n",
        app.name
    )
}

fn render_yaml(app: &AppRecord) -> String {
    format!(
        "# Render blueprint for {id} — https://render.com/docs/blueprint-spec\n\
         services:\n\
         \x20 - type: web\n\
         \x20   name: {id}\n\
         \x20   runtime: docker\n\
         \x20   plan: starter\n\
         \x20   healthCheckPath: /health\n\
         \x20   envVars:\n\
         \x20     - key: HIPAA_CORE\n\
         \x20       value: enabled\n\
         \x20     - key: APP_VERSION\n\
         \x20       value: \"{version}\"\n",
        id = app.id,
        version = app.current_version
    )
}

fn fly_toml(app: &AppRecord) -> String {
    format!(
        "# Fly.io config for {id}\n\
         app = \"{id}\"\n\
         primary_region = \"ewr\"\n\n\
         [build]\n\
         \x20 dockerfile = \"Dockerfile\"\n\n\
         [env]\n\
         \x20 HIPAA_CORE = \"enabled\"\n\
         \x20 APP_VERSION = \"{version}\"\n\n\
         [http_service]\n\
         \x20 internal_port = 8080\n\
         \x20 force_https = true\n\n\
         [[http_service.checks]]\n\
         \x20 method = \"GET\"\n\
         \x20 path = \"/health\"\n\
         \x20 interval = \"10s\"\n\
         \x20 timeout = \"2s\"\n",
        id = app.id,
        version = app.current_version
    )
}

fn kamal_deploy_yml(app: &AppRecord) -> String {
    format!(
        "# Kamal deploy config for {id} — https://kamal-deploy.org\n\
         service: {id}\n\
         image: {tenant}/{id}\n\
         servers:\n\
         \x20 web:\n\
         \x20   - 192.0.2.10 # replace with your host\n\
         proxy:\n\
         \x20 app_port: 8080\n\
         \x20 healthcheck:\n\
         \x20   path: /health\n\
         registry:\n\
         \x20 server: ghcr.io\n\
         \x20 username: {tenant}\n\
         \x20 password:\n\
         \x20   - KAMAL_REGISTRY_PASSWORD\n\
         env:\n\
         \x20 clear:\n\
         \x20   HIPAA_CORE: enabled\n\
         \x20   APP_VERSION: \"{version}\"\n",
        id = app.id,
        tenant = app.tenant,
        version = app.current_version
    )
}

/// Live apps ship the platform's own rendered allocation spec; sandbox apps
/// get a stub that says exactly how to earn the real one.
fn nomad_job(app: &AppRecord) -> String {
    deploy::render_job(app).unwrap_or_else(|_| {
        format!(
            "# app {} has no live allocation yet — the Nomad job is rendered from\n\
             # the live allocation at promotion (green gate report + co-signature).\n\
             # Promote the app, then re-export this bundle for the real job spec.\n",
            app.id
        )
    })
}

// ---------- pack.hcl: the app as the doctor's own template ----------

/// Derive a pack manifest from the app: scaffold = what they built, gates =
/// what their pack demanded, prewired = the controls they actually wired that
/// are gates. Parses with the platform's own `packs::parse_pack`.
fn derived_pack_hcl(app: &AppRecord, pack: &PackManifest) -> String {
    let prewired: Vec<&str> = app
        .controls
        .iter()
        .filter(|c| pack.gates.contains(*c))
        .map(String::as_str)
        .collect();
    let description = format!(
        "Template derived from {} (built from: {})",
        app.name, app.prompt
    );
    format!(
        "// Derived from app \"{id}\" at ejection — the doctor's own re-usable template.\n\
         // Same schema as the platform's packs/*/pack.hcl; re-import or share as-is.\n\n\
         pack \"{id}-template\" {{\n\
         \x20 name        = \"{name}\"\n\
         \x20 description = \"{description}\"\n\
         \x20 profile     = \"{profile}\"\n\
         \x20 tier        = {tier}\n\
         \x20 wave        = {wave}\n\
         \x20 signed_by   = \"platform-root-v1\"\n\n\
         \x20 # What this template scaffolds: the app's features as built, v{version}.\n\
         \x20 scaffold = [\n{scaffold}\x20 ]\n\n\
         \x20 # Controls wired at ejection that the gate set checks.\n\
         \x20 prewired = [\n{prewired}\x20 ]\n\n\
         \x20 # Gates that must be green before promotion, carried from pack {pack_id}.\n\
         \x20 gates = [\n{gates}\x20 ]\n\n\
         \x20 synthetic_dataset = \"{dataset}\"\n\
         }}\n",
        id = app.id,
        name = hcl_str(&format!("{} (template)", app.name)),
        description = hcl_str(&description),
        profile = hcl_str(&pack.profile),
        tier = pack.tier,
        wave = pack.wave,
        version = app.current_version,
        scaffold = hcl_list(app.features.iter().map(String::as_str)),
        prewired = hcl_list(prewired.iter().copied()),
        gates = hcl_list(pack.gates.iter().map(String::as_str)),
        pack_id = pack.id,
        dataset = hcl_str(&pack.synthetic_dataset),
    )
}

fn hcl_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items
        .map(|item| format!("    \"{}\",\n", hcl_str(item)))
        .collect()
}

/// Escape a string for an HCL quoted literal: quotes, backslashes, newlines,
/// and template interpolation sequences.
fn hcl_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.replace("${", "$${").replace("%{", "%%{")
}

/// Markdown table cell: keep pipes and newlines from breaking the row.
fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// Unix seconds → "YYYY-MM-DD HH:MM UTC" (Howard Hinnant's civil-from-days),
/// so the generated record is readable without a date dependency.
fn utc(ts: u64) -> String {
    let days = (ts / 86_400) as i64;
    let secs = ts % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        secs / 3_600,
        (secs % 3_600) / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates;
    use crate::packs;
    use crate::state::{now_unix, Addendum};
    use std::collections::BTreeSet;

    fn post_op_pack() -> PackManifest {
        packs::builtin_packs()
            .into_iter()
            .find(|p| p.id == "post-op-monitor")
            .unwrap()
    }

    fn sample_app(pack: &PackManifest) -> AppRecord {
        let mut features = pack.scaffold.clone();
        // A feature with characters that must survive HCL escaping.
        features.push("pain 0-10 scale with a \"flag over 7\" rule".to_string());
        AppRecord {
            id: "post-op-tracker".to_string(),
            name: "post-op tracker".to_string(),
            prompt: "a post-op recovery tracker for my knee replacement patients".to_string(),
            pack: pack.id.clone(),
            stage: Stage::Sandbox,
            data_source: DataSource::Synthetic(pack.synthetic_dataset.clone()),
            controls: pack.gates.iter().cloned().collect::<BTreeSet<_>>(),
            external_calls: vec!["api.anthropic.com".to_string()],
            features,
            routes: 5,
            addenda: vec![Addendum {
                version: 1,
                instruction: "initial draft from protocol".to_string(),
                reply: "scaffolded from pack post-op-monitor".to_string(),
                added_feature: None,
                wired_controls: pack.prewired.clone(),
                at: now_unix(),
            }],
            current_version: 1,
            reviewer_note: None,
            allocation: None,
            attestation: None,
            tenant: "meridian".to_string(),
        }
    }

    #[test]
    fn derived_pack_round_trips_through_the_platform_parser() {
        let pack = post_op_pack();
        let app = sample_app(&pack);
        let hcl = derived_pack_hcl(&app, &pack);

        let template = packs::parse_pack(&hcl).expect("derived pack.hcl must parse and verify");
        assert_eq!(template.id, "post-op-tracker-template");
        assert_eq!(template.signed_by, "platform-root-v1");
        assert_eq!(
            template.scaffold, app.features,
            "scaffold = features as built"
        );
        assert_eq!(template.gates, pack.gates, "gates carried from the pack");
        assert_eq!(template.synthetic_dataset, pack.synthetic_dataset);
        assert_eq!(template.tier, pack.tier);
        // prewired = wired controls that are also gates, and nothing else.
        for control in &template.prewired {
            assert!(pack.gates.contains(control));
            assert!(app.controls.contains(control));
        }
        assert_eq!(template.prewired.len(), pack.gates.len());
    }

    #[test]
    fn sandbox_bundle_is_draft_with_no_attestation_and_stub_job() {
        let pack = post_op_pack();
        let app = sample_app(&pack);
        let bundle = bundle(&app, &pack, &[]);

        let compliance = &bundle.files["docs/COMPLIANCE.md"];
        assert!(compliance.contains("draft — not released"));
        assert!(compliance.contains("omitted by design"));
        assert!(!compliance.contains("co-signed by:"));
        assert!(bundle.files["nomad/job.nomad.hcl"].contains("no live allocation yet"));
        assert!(bundle.unpack.contains("python3"));
    }

    #[test]
    fn live_bundle_embeds_attestation_and_rendered_job() {
        let pack = post_op_pack();
        let mut app = sample_app(&pack);
        let report = gates::preflight(&app, &pack.gates);
        assert!(report.green, "sample app should pass its gates");
        deploy::promote(&mut app, &report, "Dr. A. Osei", "a-0001".to_string())
            .expect("promotion succeeds on a green report");

        let bundle = bundle(&app, &pack, &[]);
        let compliance = &bundle.files["docs/COMPLIANCE.md"];
        assert!(compliance.contains("Status: **released**"));
        assert!(compliance.contains("**Dr. A. Osei**"));
        assert!(compliance.contains("**6/6**"));
        assert!(
            compliance.contains("6/6 checks passed — green"),
            "the re-run gate report must reflect the released record, not fail synthetic-only: {compliance}"
        );
        assert!(bundle.files["nomad/job.nomad.hcl"].contains("job \"post-op-tracker\""));
        assert!(bundle.files["README.md"].contains("knee replacement patients"));
    }
}
