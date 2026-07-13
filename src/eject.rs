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
use std::collections::{BTreeMap, BTreeSet};

use crate::audit::AuditEvent;
use crate::deploy;
use crate::gates::{self, GateReport, GateStatus};
use crate::packs::PackManifest;
use crate::state::{AppRecord, DataSource, Stage};

/// Writes the file-map to disk from stdin. Stock python3, no dependencies —
/// pipe the export JSON through it inside the target directory.
pub const UNPACK_ONE_LINER: &str = r#"python3 -c 'import json,sys,pathlib; root=pathlib.Path.cwd().resolve(); files=json.load(sys.stdin)["files"]; [(lambda q,c: (_ for _ in ()).throw(ValueError("unsafe export path")) if q == root or root not in q.parents else (q.parent.mkdir(parents=True,exist_ok=True),q.open("x",encoding="utf-8").write(c)))((root/pathlib.Path(p)).resolve(),c) for p,c in files.items()]'"#;

#[derive(Clone, Debug, Serialize)]
pub struct EjectionBundle {
    /// Relative path → file content. BTreeMap so the listing is stable.
    pub files: BTreeMap<String, String>,
    /// Copy-paste command that unpacks this bundle into the current directory.
    pub unpack: String,
}

/// Validate the inert file map before the control plane verifies or stores an
/// owned import. This is a source contract, not a signer or release decision.
pub fn validate_owned_bundle(files: &BTreeMap<String, String>) -> Result<(), String> {
    let workspace =
        crate::workspace::WorkspaceRecord::new("owned-import-validation".into(), files.clone(), 0);
    workspace.validate_restored()?;

    let required = [
        "README.md",
        "pack.hcl",
        ".mcp.json",
        ".gitignore",
        ".dockerignore",
        "Dockerfile",
        "artifact-quality.json",
        "server/Cargo.toml",
        "server/Cargo.lock",
        "server/src/main.rs",
        "web/package.json",
        "web/package-lock.json",
        "web/src/routes/+page.svelte",
        "web/tests/owned-app.mjs",
        "scripts/reimport.mjs",
    ];
    if let Some(path) = required.iter().find(|path| !files.contains_key(**path)) {
        return Err(format!("owned bundle is missing {path}"));
    }
    if !files.keys().any(|path| path.starts_with("synthetic/")) {
        return Err("owned bundle is missing a synthetic fixture".into());
    }
    if files.keys().any(|path| {
        path.starts_with("docs/")
            || path == ".practice-verifier-report.json"
            || path.ends_with("/.practice-verifier-report.json")
            || path.starts_with("server/target/")
            || path.starts_with("web/node_modules/")
            || path.starts_with("web/build/")
            || path.starts_with("web/test-results/")
    }) {
        return Err("owned bundle contains a reserved path".into());
    }
    let markdown = files
        .keys()
        .filter(|path| path.ends_with(".md"))
        .collect::<Vec<_>>();
    if markdown.len() != 1 || markdown[0].as_str() != "README.md" {
        return Err("owned bundle must contain README.md as its only Markdown file".into());
    }
    let diagrams = files
        .keys()
        .filter(|path| path.ends_with(".tldr"))
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "diagrams/service-map.tldr",
        "diagrams/system-architecture.tldr",
        "diagrams/workspace-state-machine.tldr",
    ]);
    if diagrams != expected {
        return Err("owned bundle must contain the three required tldraw diagrams".into());
    }
    let mcp: serde_json::Value = serde_json::from_str(&files[".mcp.json"])
        .map_err(|_| ".mcp.json is not valid JSON".to_string())?;
    if mcp["mcpServers"]["svelte"]["url"] != "https://mcp.svelte.dev/mcp" {
        return Err("owned bundle must keep the official Svelte MCP endpoint".into());
    }
    Ok(())
}

/// Build the ejection bundle. Pure over its inputs: the same record and
/// audit slice always produce the same bundle, so an export is evidence.
pub fn bundle(app: &AppRecord, pack: &PackManifest, audit: &[&AuditEvent]) -> EjectionBundle {
    let (report, provenance) = preflight_report(app, pack);
    // Packs converted to the full spec (#5) ship a runnable scaffold whose
    // sources are compile-time embedded next to the manifests (packs.rs) —
    // the same no-drift guarantee PACK_SOURCES gives. `scaffold_path` on the
    // signed manifest is the opt-in; the embedded table is the content.
    let scaffold = pack
        .scaffold_path
        .as_deref()
        .and_then(|_| crate::packs::scaffold_sources(&pack.id));
    let clinical_entry = clinical_entry_path(scaffold);
    let mut files = BTreeMap::new();
    if let Some(sources) = scaffold {
        for (path, content) in sources {
            // scaffold/* becomes the bundle's server/ source tree; everything
            // else (the synthetic seed) keeps its pack-relative path so the
            // app's `../synthetic/…` loading and `include_str!` both resolve.
            let dest = match path.strip_prefix("scaffold/") {
                Some(rest) => format!("server/{rest}"),
                None => (*path).to_string(),
            };
            files.insert(dest, (*content).to_string());
        }
        files.insert(
            "web/src/clinician.css".to_string(),
            clinician_design_system_css().to_string(),
        );
    }
    files.insert("web/package.json".to_string(), svelte_package_json());
    files.insert(
        "web/package-lock.json".to_string(),
        svelte_package_lock().to_string(),
    );
    files.insert("web/svelte.config.js".to_string(), svelte_config());
    files.insert("web/vite.config.ts".to_string(), svelte_vite_config());
    files.insert("web/tsconfig.json".to_string(), svelte_tsconfig());
    files.insert("web/src/app.html".to_string(), svelte_app_html());
    files.insert("web/src/app.d.ts".to_string(), svelte_app_types());
    files.insert(
        "web/tests/owned-app.mjs".to_string(),
        owned_app_browser_test(pack),
    );
    files.insert("web/src/routes/+layout.svelte".to_string(), svelte_layout());
    files.insert(
        "web/src/routes/+layout.ts".to_string(),
        svelte_layout_options(),
    );
    files.insert(
        "web/src/routes/+page.svelte".to_string(),
        svelte_page(app, pack, &clinical_entry),
    );
    files.insert(
        "web/src/lib/treatment.json".to_string(),
        default_treatment_config(),
    );
    if pack.id == "post-op-monitor" {
        files.insert(
            "web/src/lib/PostOpCheckIn.svelte".to_string(),
            post_op_checkin_component(),
        );
    }
    files.insert(".mcp.json".to_string(), svelte_mcp_config());
    files.insert("scripts/reimport.mjs".to_string(), owned_reimport_script());
    files.insert(".gitignore".to_string(), exported_gitignore());
    files.insert(".dockerignore".to_string(), exported_dockerignore());
    files.insert(
        "diagrams/system-architecture.tldr".to_string(),
        tldraw_diagram(
            "System architecture",
            &[
                "Svelte 5 web",
                "Rust and Axum server",
                "Synthetic data",
                "Owned deployment",
            ],
        ),
    );
    files.insert(
        "diagrams/workspace-state-machine.tldr".to_string(),
        tldraw_diagram(
            "Workspace state machine",
            &[
                "Describe",
                "Choose treatment",
                "Review diff",
                "Verify",
                "Accept",
                "Export",
            ],
        ),
    );
    files.insert(
        "diagrams/service-map.tldr".to_string(),
        tldraw_diagram(
            "Service map",
            &[
                "Clinician",
                "Svelte client",
                "Rust API",
                "Gemma planner",
                "Gate",
                "Runtime",
            ],
        ),
    );
    let mut readme = readme_md(app, pack);
    readme.push_str("\n\n");
    readme.push_str(
        &runbook_md(app, pack, scaffold.is_some())
            .replace("app/", "server/")
            .replace("cd app", "cd server")
            .replace("docs/COMPLIANCE.md", "the compliance section below"),
    );
    readme.push_str("\n\n");
    readme.push_str(
        &customize_md(app, pack, scaffold)
            .replace("app/", "server/")
            .replace("cd app", "cd server")
            .replace("docs/DESIGN_SYSTEM.md", "the design system section below")
            .replace("docs/COMPLIANCE.md", "the compliance section below")
            .replace("docs/RUNBOOK.md", "the run instructions above"),
    );
    readme.push_str("\n\n");
    readme
        .push_str(&design_system_md().replace("app/assets/clinician.css", "web/src/clinician.css"));
    readme.push_str("\n\n");
    readme.push_str(&compliance_md(app, &report, provenance, audit));
    readme.push_str("\n\n## Extend with Gemma planning\n\nThe platform uses Gemma as its only application model. The exported app does not phone home to that planner. The `.mcp.json` file connects compatible editors to the official Svelte MCP server for API documentation and component repair; it is not a second model runtime. If you connect your own Gemma endpoint, keep it behind the Rust API. Do not give it production secrets, deployment authority, file access, or patient data.\n");
    files.insert("README.md".to_string(), readme);
    files.insert("Dockerfile".to_string(), dockerfile(app, scaffold));
    files.insert(
        "config/nginx.conf".to_string(),
        nginx_config(&clinical_entry),
    );
    files.insert("config/start.sh".to_string(), runtime_start_script());
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

/// Where the compliance record's gate report came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReportProvenance {
    /// Frozen verbatim at promotion (F3): the report that admitted the app.
    Frozen,
    /// Re-run live at export — the draft path, where preflight is current.
    Rerun,
}

/// The gate report for the compliance record (F3, review-log round 1):
/// a released app embeds the report frozen on its attestation at promotion
/// — the evidence that admitted it — never a re-run over reconstructed
/// lineage. Draft bundles keep the live re-run, where preflight is simply
/// the current truth. The lineage-reconstruction fallback survives only for
/// live records that predate stored reports (a released app legitimately
/// reads tenant data and would fail synthetic-only forever if re-run raw).
fn preflight_report(app: &AppRecord, pack: &PackManifest) -> (GateReport, ReportProvenance) {
    if app.stage == Stage::Live {
        if let Some(report) = app.attestation.as_ref().and_then(|a| a.report.clone()) {
            return (report, ReportProvenance::Frozen);
        }
        let mut release_view = app.clone();
        release_view.data_source = DataSource::Synthetic(pack.synthetic_dataset.clone());
        (
            gates::preflight(&release_view, &pack.gates),
            ReportProvenance::Rerun,
        )
    } else {
        (gates::preflight(app, &pack.gates), ReportProvenance::Rerun)
    }
}

fn unpack_command(app_id: &str) -> String {
    format!("mkdir -p {app_id} && cd {app_id} && {UNPACK_ONE_LINER} < ../export.json")
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

    md.push_str("## Repository map\n\n");
    md.push_str(&format!(
        "- `web/` contains the Svelte 5 interface; `web/src/lib/treatment.json` is the Rust-materialized treatment you can keep editing.\n\
         - `server/` contains the Rust and Axum service.\n\
         - `synthetic/` contains safe example data.\n\
         - `diagrams/` contains editable tldraw system, state, and service diagrams.\n\
         - `.mcp.json` connects an editor to Svelte MCP.\n\
         - `pack.hcl` is this app as your own template (`{}-template`).\n",
        app.id
    ));
    md
}

fn svelte_package_json() -> String {
    r#"{
  "name": "owned-clinical-tool-web",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite dev",
    "build": "vite build",
    "check": "svelte-kit sync && svelte-check --tsconfig ./tsconfig.json",
    "test:journey": "node tests/owned-app.mjs"
  },
  "devDependencies": {
    "@sveltejs/adapter-static": "3.0.10",
    "@sveltejs/kit": "2.69.2",
    "@sveltejs/vite-plugin-svelte": "7.2.0",
    "playwright": "1.61.1",
    "svelte": "5.56.4",
    "svelte-check": "4.4.5",
    "typescript": "5.9.3",
    "vite": "8.1.4"
  }
}
"#
    .to_string()
}

fn owned_app_browser_test(pack: &PackManifest) -> String {
    r#"import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const baseUrl = (process.env.OWNED_APP_URL || 'http://127.0.0.1:8080').replace(/\/$/, '');
const postOp = __POST_OP__;
const resultDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', 'test-results');
const allowedHosts = new Set(['127.0.0.1', 'localhost', new URL(baseUrl).hostname]);
fs.rmSync(resultDir, { recursive: true, force: true });
fs.mkdirSync(resultDir, { recursive: true });

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
const page = await context.newPage();
const externalHosts = new Set();
page.on('request', (request) => {
  const host = new URL(request.url()).hostname;
  if (!allowedHosts.has(host)) externalHosts.add(host);
});

try {
  const health = await context.request.get(`${baseUrl}/health`);
  assert.equal(health.ok(), true, 'Rust health check failed');
  assert.equal((await health.json()).status, 'ok', 'Rust returned an unhealthy status');

  const workspace = await page.goto(`${baseUrl}/workspace/`, { waitUntil: 'networkidle' });
  assert.equal(workspace?.ok(), true, 'Svelte workspace did not load');
  await page.getByText('Rust service connected', { exact: true }).waitFor();
  await page.screenshot({ path: path.join(resultDir, 'workspace.png'), fullPage: true });

  if (postOp) {
    await page.getByRole('button', { name: 'Pain 8', exact: true }).click();
    await page.getByRole('button', { name: 'Send today’s check-in', exact: true }).click();
    const patientLink = page.getByRole('link', { name: 'Sign in as the synthetic patient', exact: true });
    await patientLink.waitFor();
    await patientLink.click();
    await page.getByLabel('username').fill('demo-patient');
    await page.getByLabel('password').fill('learn-patient');
    await page.getByRole('button', { name: 'sign in', exact: false }).click();
    await page.waitForURL(`${baseUrl}/workspace/`);

    await page.getByRole('button', { name: 'Pain 8', exact: true }).click();
    await page.getByRole('button', { name: 'Send today’s check-in', exact: true }).click();
    await page.getByText('Queued in the synthetic practice inbox.', { exact: true }).waitFor();
    await page.getByText(/Pain 8\/10 was evaluated by Rust/).waitFor();
    await page.screenshot({ path: path.join(resultDir, 'patient-escalation.png'), fullPage: true });

    await page.goto(`${baseUrl}/login`, { waitUntil: 'domcontentloaded' });
    await page.getByLabel('username').fill('demo-clinician');
    await page.getByLabel('password').fill('learn-clinician');
    await page.getByRole('button', { name: 'sign in', exact: false }).click();
    await page.waitForURL(`${baseUrl}/clinician`);
    await page.getByText(/pain 8\/10 at or over threshold 7/).first().waitFor();
    await page.screenshot({ path: path.join(resultDir, 'clinician-inbox.png'), fullPage: true });
  }

  assert.deepEqual([...externalHosts], [], `The app contacted external hosts: ${[...externalHosts].join(', ')}`);
  fs.writeFileSync(path.join(resultDir, 'report.json'), `${JSON.stringify({ passed: true, baseUrl, postOp }, null, 2)}\n`);
  console.log(`Owned app journey passed. Evidence is in ${resultDir}`);
} catch (error) {
  await page.screenshot({ path: path.join(resultDir, 'failure.png'), fullPage: true }).catch(() => {});
  fs.writeFileSync(path.join(resultDir, 'report.json'), `${JSON.stringify({ passed: false, error: String(error) }, null, 2)}\n`);
  throw error;
} finally {
  await context.close();
  await browser.close();
}
"#
    .replace("__POST_OP__", if pack.id == "post-op-monitor" { "true" } else { "false" })
}

fn exported_gitignore() -> String {
    "reimport-result.json\nserver/target/\nweb/.svelte-kit/\nweb/build/\nweb/node_modules/\nweb/test-results/\n"
        .to_string()
}

fn exported_dockerignore() -> String {
    ".git\nreimport-result.json\nserver/target\nweb/.svelte-kit\nweb/build\nweb/node_modules\nweb/test-results\n"
        .to_string()
}

fn owned_reimport_script() -> String {
    r#"import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const baseUrl = (process.env.PRACTICE_STUDIO_URL || 'http://127.0.0.1:3000').replace(/\/$/, '');
const token = process.env.PRACTICE_STUDIO_TOKEN;
const ignored = new Set([
  '.git',
  'server/target',
  'web/.svelte-kit',
  'web/build',
  'web/node_modules',
  'web/test-results',
]);
const files = {};

function walk(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const absolute = path.join(directory, entry.name);
    const relative = path.relative(root, absolute).split(path.sep).join('/');
    if (ignored.has(relative) || relative === '.DS_Store' || relative === 'reimport-result.json') continue;
    if (entry.isSymbolicLink()) throw new Error(`Refusing symbolic link: ${relative}`);
    if (entry.isDirectory()) walk(absolute);
    else if (entry.isFile()) files[relative] = fs.readFileSync(absolute, 'utf8');
    else throw new Error(`Refusing non-file entry: ${relative}`);
  }
}

walk(root);
const headers = { 'content-type': 'application/json' };
if (token) headers.authorization = `Bearer ${token}`;
const response = await fetch(`${baseUrl}/api/apps/import`, {
  method: 'POST',
  headers,
  body: JSON.stringify({ files }),
});
const body = await response.json();
if (!response.ok) throw new Error(body.error || `Import failed with HTTP ${response.status}`);
fs.writeFileSync(path.join(root, 'reimport-result.json'), `${JSON.stringify(body, null, 2)}\n`);
console.log(`Imported ${body.app.id} as a private synthetic starter.`);
console.log(`Open ${baseUrl}/ and select ${body.app.id}.`);
"#
    .to_string()
}

fn svelte_package_lock() -> &'static str {
    include_str!("../export-assets/web-package-lock.json")
}

fn svelte_config() -> String {
    "import adapter from '@sveltejs/adapter-static';\n\nexport default {\n  kit: {\n    adapter: adapter({ fallback: 'index.html' }),\n    paths: { base: '/workspace' }\n  }\n};\n".to_string()
}

fn svelte_vite_config() -> String {
    "import { sveltekit } from '@sveltejs/kit/vite';\nimport { defineConfig } from 'vite';\n\nexport default defineConfig({\n  plugins: [sveltekit()],\n  server: { proxy: { '/health': 'http://127.0.0.1:8080', '/api': 'http://127.0.0.1:8080' } }\n});\n".to_string()
}

fn svelte_tsconfig() -> String {
    "{\n  \"extends\": \"./.svelte-kit/tsconfig.json\",\n  \"compilerOptions\": {\n    \"allowJs\": true,\n    \"checkJs\": true,\n    \"esModuleInterop\": true,\n    \"forceConsistentCasingInFileNames\": true,\n    \"resolveJsonModule\": true,\n    \"skipLibCheck\": true,\n    \"sourceMap\": true,\n    \"strict\": true,\n    \"moduleResolution\": \"bundler\"\n  }\n}\n".to_string()
}

fn svelte_app_html() -> String {
    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    %sveltekit.head%\n  </head>\n  <body data-sveltekit-preload-data=\"hover\">\n    <div style=\"display: contents\">%sveltekit.body%</div>\n  </body>\n</html>\n".to_string()
}

fn svelte_app_types() -> String {
    "declare global { namespace App {} }\nexport {};\n".to_string()
}

fn svelte_layout() -> String {
    "<script lang=\"ts\">\n  import '../clinician.css';\n  let { children } = $props();\n</script>\n\n{@render children()}\n".to_string()
}

fn svelte_layout_options() -> String {
    "export const ssr = false;\n".to_string()
}

fn svelte_page(app: &AppRecord, pack: &PackManifest, clinical_entry: &str) -> String {
    if pack.id == "post-op-monitor" {
        return post_op_svelte_page(app, pack, clinical_entry);
    }
    let features = app
        .features
        .iter()
        .map(|feature| format!("    {{ label: {:?}, done: true }}", feature))
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        r#"<script lang="ts">
  import {{ onMount }} from 'svelte';
  import treatment from '../lib/treatment.json';
  let items = $state([
{features}
  ]);
  let note = $state('');
  let rustStatus = $state('Checking the Rust service');

  onMount(async () => {{
    try {{
      const response = await fetch('/health', {{ headers: {{ accept: 'application/json' }} }});
      const contentType = response.headers.get('content-type') ?? '';
      const health = contentType.includes('application/json') ? await response.json() : null;
      rustStatus = response.ok && health?.status === 'ok'
        ? 'Rust service connected'
        : 'Rust service needs attention';
    }} catch {{
      rustStatus = 'Start the Rust service to connect this workspace';
    }}
  }});

  function addItem() {{
    const label = note.trim();
    if (!label) return;
    items.push({{ label, done: false }});
    note = '';
  }}
</script>

<svelte:head><title>{name}</title></svelte:head>

<main class="hc-page">
  <div class="hc-shell hc-stack">
    <header class="hc-card hc-stack">
      <span class="hc-badge">Synthetic learning environment</span>
      <h1>{name}</h1>
      <p>{description}</p>
      <div class="hc-actions">
        <a class="hc-button hc-link" href="{clinical_entry}">Open the clinical workflow</a>
        <span class="hc-badge" data-rust-status>{{rustStatus}}</span>
      </div>
    </header>
    {{#if treatment.refinement.presentation === 'context-first'}}
      <section class="hc-card hc-stack" data-testid="treatment-context">
        <span class="hc-badge">{{treatment.treatment.label}}</span>
        <h2>{{treatment.treatment.user_outcome}}</h2>
        {{#if treatment.refinement.emphasis}}<p>{{treatment.refinement.emphasis}}</p>{{/if}}
        <p class="hc-help">Planned by {{treatment.planner.model}} · materialized by Rust</p>
      </section>
    {{/if}}
    <section class="hc-card hc-stack" aria-labelledby="workflow-title">
      <h2 id="workflow-title">Current workflow</h2>
      {{#each items as item}}
        <label class="hc-actions"><input type="checkbox" bind:checked={{item.done}} /> {{item.label}}</label>
      {{/each}}
      <label class="hc-label">Add a synthetic workflow step
        <span class="hc-help">Do not enter patient information.</span>
        <input class="hc-field" bind:value={{note}} onkeydown={{(event) => event.key === 'Enter' && addItem()}} />
      </label>
      <button class="hc-button hc-button--primary" onclick={{addItem}}>Add step</button>
      <aside class="hc-notice hc-notice--warning" role="note">This starter is not monitored for emergencies or approved for clinical care.</aside>
    </section>
    {{#if treatment.refinement.presentation === 'task-first'}}
      <section class="hc-card hc-stack" data-testid="treatment-context">
        <span class="hc-badge">{{treatment.treatment.label}}</span>
        <h2>{{treatment.treatment.user_outcome}}</h2>
        {{#if treatment.refinement.emphasis}}<p>{{treatment.refinement.emphasis}}</p>{{/if}}
        <p class="hc-help">Planned by {{treatment.planner.model}} · materialized by Rust</p>
      </section>
    {{/if}}
  </div>
</main>
"#,
        name = app.name,
        description = pack.description,
        clinical_entry = clinical_entry,
    )
}

fn post_op_svelte_page(app: &AppRecord, pack: &PackManifest, clinical_entry: &str) -> String {
    format!(
        r#"<script lang="ts">
  import {{ onMount }} from 'svelte';
  import PostOpCheckIn from '../lib/PostOpCheckIn.svelte';
  let rustStatus = $state('Checking the Rust service');

  onMount(async () => {{
    try {{
      const response = await fetch('/health', {{ headers: {{ accept: 'application/json' }} }});
      const health = response.headers.get('content-type')?.includes('application/json')
        ? await response.json()
        : null;
      rustStatus = response.ok && health?.status === 'ok'
        ? 'Rust service connected'
        : 'Rust service needs attention';
    }} catch {{
      rustStatus = 'Start the Rust service to connect this workspace';
    }}
  }});
</script>

<svelte:head><title>{name}</title></svelte:head>

<main class="hc-page">
  <div class="hc-shell hc-stack">
    <header class="hc-card hc-stack">
      <div class="hc-actions">
        <span class="hc-badge">Synthetic learning environment</span>
        <span class="hc-badge" data-rust-status>{{rustStatus}}</span>
      </div>
      <h1>{name}</h1>
      <p>{description}</p>
      <p class="hc-help">This editable Svelte screen submits to the same-origin Rust service. Rust validates the check-in, decides whether escalation is required, and writes the audit trail.</p>
    </header>
    <PostOpCheckIn />
    <footer class="hc-card hc-actions">
      <a class="hc-button hc-link" href="{clinical_entry}">Open the role-based clinical view</a>
      <span class="hc-help">Edit this screen in <code>web/src/lib/PostOpCheckIn.svelte</code>. The API contract lives in <code>server/src/main.rs</code>.</span>
    </footer>
  </div>
</main>
"#,
        name = app.name,
        description = pack.description,
        clinical_entry = clinical_entry,
    )
}

fn post_op_checkin_component() -> String {
    r#"<script lang="ts">
  import treatment from './treatment.json';

  type CheckinResult = {
    status: 'recorded';
    synthetic: true;
    replayed: boolean;
    checkin_id: string;
    pain: number;
    wound: string;
    escalation: {
      required: boolean;
      flag_id: string | null;
      destination: 'practice-inbox' | null;
      status: 'queued' | 'not-required';
      reason_codes: string[];
    };
    message: string;
  };

  let pain = $state(8);
  let wound = $state('clean');
  let note = $state('Synthetic example: pain increased overnight.');
  let submitting = $state(false);
  let result: CheckinResult | null = $state(null);
  let error = $state('');
  let needsSignIn = $state(false);
  let idempotencyKey = '';

  async function submitCheckin() {
    if (submitting) return;
    submitting = true;
    result = null;
    error = '';
    needsSignIn = false;
    idempotencyKey ||= crypto.randomUUID().replaceAll('-', '');
    try {
      const response = await fetch('/api/checkins', {
        method: 'POST',
        credentials: 'same-origin',
        headers: {
          accept: 'application/json',
          'content-type': 'application/json',
          'idempotency-key': idempotencyKey
        },
        body: JSON.stringify({ pain, wound, note })
      });
      if (response.status === 401) {
        needsSignIn = true;
        throw new Error('Sign in as the synthetic patient to submit this check-in.');
      }
      const body = await response.json();
      if (!response.ok) throw new Error(body.message || body.error || 'The check-in was not recorded.');
      result = body as CheckinResult;
      idempotencyKey = '';
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'The check-in was not recorded.';
    } finally {
      submitting = false;
    }
  }
</script>

<section class="hc-stack" aria-labelledby="checkin-title">
  {#if treatment.refinement.presentation === 'context-first'}
    <article class="hc-card hc-stack" data-testid="treatment-context">
      <div class="hc-actions"><span class="hc-badge">Gemma-planned treatment</span><b>{treatment.treatment.label}</b></div>
      <h2>{treatment.treatment.user_outcome}</h2>
      {#if treatment.refinement.emphasis}<p>{treatment.refinement.emphasis}</p>{/if}
      <div class="hc-notice"><b>What happens next</b><p>Concerning synthetic answers are evaluated by Rust and, when required, queued to the practice inbox with a visible reason.</p></div>
      <p class="hc-help">Planned by {treatment.planner.model} · materialized by Rust · the clinical threshold is unchanged.</p>
    </article>
  {/if}
  <div class="hc-workflow-grid">
  <form class="hc-card hc-stack" onsubmit={(event) => { event.preventDefault(); submitCheckin(); }}>
    <div class="hc-actions">
      <span class="hc-badge">Synthetic practice patient</span>
      <span class="hc-help">Today’s recovery check-in</span>
    </div>
    <h2 id="checkin-title">Meridian Recovery Check-in</h2>
    <p class="hc-help">Try pain 8 to see the Rust escalation rule route one synthetic flag.</p>

    <label class="hc-label">pain (0–10)
      <input class="hc-field" type="range" min="0" max="10" bind:value={pain} aria-describedby="pain-value" />
    </label>
    <div class="hc-pain-scale" aria-label="Choose a pain score">
      {#each Array.from({ length: 11 }, (_, value) => value) as value}
        <button type="button" class:active={pain === value} aria-label={`Pain ${value}`} aria-pressed={pain === value} onclick={() => pain = value}>{value}</button>
      {/each}
    </div>
    <p id="pain-value" class="hc-pain-value"><b>{pain}/10</b> selected</p>

    <label class="hc-label">wound looks
      <select class="hc-field" bind:value={wound}>
        <option value="clean">clean</option>
        <option value="redness">redness</option>
        <option value="swelling">swelling</option>
        <option value="drainage">drainage</option>
        <option value="opening">opening</option>
        <option value="spreading-redness">spreading redness</option>
      </select>
    </label>
    <label class="hc-label">note <span class="hc-help">Synthetic examples only. Maximum 1,000 bytes.</span>
      <textarea class="hc-field" rows="3" maxlength="1000" bind:value={note}></textarea>
    </label>
    <button class="hc-button hc-button--primary" type="submit" disabled={submitting}>
      {submitting ? 'Sending to Rust…' : 'Send today’s check-in'}
    </button>
    <p class="hc-help">Learning environment only. This inbox is not monitored for emergencies.</p>
  </form>

  <aside class="hc-card hc-stack" aria-live="polite">
    <span class="hc-badge">Rust-owned result</span>
    <h2>Practice inbox routing</h2>
    {#if result?.escalation.required}
      <div class="hc-notice hc-notice--success" data-testid="escalation-result">
        <b>Queued in the synthetic practice inbox.</b>
        <p>Pain {result.pain}/10 was evaluated by Rust and produced flag <code>{result.escalation.flag_id}</code>.</p>
      </div>
    {:else if result}
      <div class="hc-notice" data-testid="recorded-result">
        <b>Check-in recorded.</b>
        <p>Rust determined that no escalation was required.</p>
      </div>
    {:else if error}
      <div class="hc-notice hc-notice--warning" role="alert">
        <b>{error}</b>
        {#if needsSignIn}<p><a href="/login?next=/workspace/">Sign in as the synthetic patient</a>, then return here.</p>{/if}
      </div>
    {:else}
      <div class="hc-empty-result">
        <b>No client-side guess.</b>
        <p>Submit the synthetic check-in. This panel changes only after the Rust API confirms the result.</p>
      </div>
    {/if}
    <div class="hc-boundary-list">
      <span>✓ Patient identity comes from the HttpOnly session.</span>
      <span>✓ Rust owns validation and the pain threshold.</span>
      <span>✓ A retry key prevents duplicate flags.</span>
      <span>✓ Free-text notes are not echoed in the response.</span>
    </div>
  </aside>
  </div>
  {#if treatment.refinement.presentation === 'task-first'}
    <article class="hc-card hc-stack" data-testid="treatment-context">
      <div class="hc-actions"><span class="hc-badge">Gemma-planned treatment</span><b>{treatment.treatment.label}</b></div>
      <h2>{treatment.treatment.user_outcome}</h2>
      {#if treatment.refinement.emphasis}<p>{treatment.refinement.emphasis}</p>{/if}
      <p class="hc-help">Planned by {treatment.planner.model} · materialized by Rust · the clinical threshold is unchanged.</p>
    </article>
  {/if}
</section>

<style>
  .hc-workflow-grid { display:grid; grid-template-columns:minmax(0,1.1fr) minmax(290px,.9fr); gap:1rem; }
  .hc-pain-scale { display:grid; grid-template-columns:repeat(11,minmax(34px,1fr)); gap:.35rem; }
  .hc-pain-scale button { min-height:44px; border:1px solid var(--hc-line); border-radius:10px; background:var(--hc-surface); color:var(--hc-ink); cursor:pointer; }
  .hc-pain-scale button.active { background:var(--hc-brand); border-color:var(--hc-brand); color:white; font-weight:800; }
  .hc-pain-value { margin:0; font-size:1.1rem; }
  .hc-empty-result { min-height:150px; display:grid; align-content:center; padding:1rem; border:1px dashed var(--hc-line); border-radius:12px; color:var(--hc-muted); }
  .hc-boundary-list { display:grid; gap:.55rem; font-size:.86rem; color:var(--hc-muted); }
  @media (max-width:760px) { .hc-workflow-grid { grid-template-columns:1fr; } .hc-pain-scale { grid-template-columns:repeat(6,1fr); } }
</style>
"#.to_string()
}

fn svelte_mcp_config() -> String {
    r#"{
  "mcpServers": {
    "svelte": {
      "url": "https://mcp.svelte.dev/mcp"
    }
  }
}
"#
    .to_string()
}

fn default_treatment_config() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": 1,
        "treatment": {
            "id": "starter-workflow",
            "label": "Starter workflow",
            "user_outcome": "Try the signed synthetic workflow before tailoring its presentation.",
            "screen_changes": ["Keep the starter workflow visible and editable."],
            "data_changes": [],
            "safety_notes": ["Synthetic learning environment only."]
        },
        "refinement": {
            "presentation": "task-first",
            "emphasis": null
        },
        "planner": {
            "provider": "deterministic",
            "model": "starter",
            "deployment_version": null,
            "fallback_reason": null
        },
        "materializer": "rust-convention-v2"
    }))
    .expect("default treatment configuration serializes")
}

fn tldraw_diagram(title: &str, labels: &[&str]) -> String {
    let mut records = vec![
        serde_json::json!({"id":"document:document","typeName":"document","name":"","meta":{}}),
        serde_json::json!({"id":"page:page","typeName":"page","name":title,"index":"a1","meta":{}}),
    ];
    for (index, label) in labels.iter().enumerate() {
        records.push(serde_json::json!({
            "id": format!("shape:node-{index}"),
            "typeName": "shape",
            "type": "geo",
            "x": 80 + (index % 3) * 260,
            "y": 100 + (index / 3) * 180,
            "rotation": 0,
            "index": format!("a{}", index + 1),
            "parentId": "page:page",
            "isLocked": false,
            "opacity": 1,
            "meta": {},
            "props": {
                "w": 210,
                "h": 100,
                "geo": "rectangle",
                "color": "violet",
                "labelColor": "black",
                "fill": "semi",
                "dash": "draw",
                "size": "m",
                "font": "draw",
                "text": label,
                "align": "middle",
                "verticalAlign": "middle",
                "growY": 0,
                "url": ""
            }
        }));
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "tldrawFileFormatVersion": 1,
        "schema": {"schemaVersion": 2, "sequences": {}},
        "records": records
    }))
    .expect("tldraw JSON serializes")
}

// ---------- RUNBOOK.md: a stranger gets it running from this alone ----------

fn runbook_md(app: &AppRecord, _pack: &PackManifest, real_source: bool) -> String {
    let id = &app.id;
    let source_section = if real_source {
        "## The app source is real\n\n\
         `app/` is the Rust and Axum service. It runs the clinical workflow from\n\
         the included `synthetic/` fixture. `web/` is the Svelte workspace for\n\
         extending that workflow. Run the Rust service for local development:\n\n\
         ```bash\n\
         cd app\n\
         APP_BIND=127.0.0.1:8080 cargo run\n\
         cargo test\n\
         ```\n\n\
         In a second terminal, run the Svelte workspace:\n\n\
         ```bash\n\
         cd web\n\
         npm ci\n\
         npm run dev -- --host 127.0.0.1\n\
         ```\n\n\
         Open `http://127.0.0.1:5173/workspace/`. The development server sends\n\
         `/health` and `/api/*` requests to Rust on port 8080.\n"
    } else {
        "## Honest caveat: the app source is a scaffold placeholder\n\n\
         Until this app's pack is converted to the runnable-scaffold spec (platform\n\
         issue #5; the post-op-monitor pack sets the pattern), this bundle does\n\
         **not** include generated application source. The Dockerfile builds a stub\n\
         that serves `/health` on port 8080 so every deploy manifest below is\n\
         exercisable end-to-end today. The record, documentation, gate report, and\n\
         manifests are real; the runtime is the placeholder.\n"
    };
    format!(
        "# Runbook — {name}\n\n\
         This bundle is self-contained. Nothing here phones home to the platform.\n\n\
         {source_section}\n\
         ## Unpack a downloaded export\n\n\
         Save the downloaded JSON as `export.json`, then run:\n\n\
         ```bash\n\
         {unpack}\n\
         ```\n\n\
         ## Run with Docker\n\n\
         ```bash\n\
         docker build -t {id} .\n\
         docker run --rm -p 8080:8080 {id}\n\
         curl --fail http://127.0.0.1:8080/health\n\
         ```\n\n\
         Open `http://127.0.0.1:8080/` for the clinical workflow. Open\n\
         `http://127.0.0.1:8080/workspace/` for the Svelte extension workspace.\n\
         Both use the same origin and the same Rust service.\n\n\
         ## Run the browser journey\n\n\
         Keep the Docker container running. In another terminal, run:\n\n\
         ```bash\n\
         cd web\n\
         npm ci\n\
         npm exec playwright install chromium\n\
         npm run test:journey\n\
         ```\n\n\
         Set `OWNED_APP_URL` if the app is not at `http://127.0.0.1:8080`.\n\
         The test saves its report and screenshots in `web/test-results/`.\n\n\
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
         ## Import as a private starter\n\n\
         Start Practice Studio, then run this command from the bundle root:\n\n\
         ```bash\n\
         PRACTICE_STUDIO_URL=http://127.0.0.1:3000 node scripts/reimport.mjs\n\
         ```\n\n\
         Set `PRACTICE_STUDIO_TOKEN` when the platform requires a bearer token.\n\
         Rust validates the file map, resolves `based_on` against the trusted built-in\n\
         registry, and verifies the exact source digest. The import receives a new id\n\
         in your tenant. It starts with synthetic data and receives no prior release,\n\
         deployment, credential, or signer authority. The command writes the result to\n\
         the ignored `reimport-result.json` file.\n",
        name = app.name,
        unpack = unpack_command(id),
        id = id,
    )
}

// ---------- CUSTOMIZE.md: the next owner can keep building ----------

fn customize_md(
    app: &AppRecord,
    pack: &PackManifest,
    scaffold: Option<&[crate::packs::PackSourceFile]>,
) -> String {
    let fixture = scaffold
        .and_then(|files| {
            files
                .iter()
                .find(|(path, _)| path.starts_with("synthetic/"))
                .map(|(path, _)| *path)
        })
        .unwrap_or("synthetic/");
    let features = app
        .features
        .iter()
        .map(|feature| format!("- {feature}"))
        .collect::<Vec<_>>()
        .join("\n");
    let gates = pack
        .gates
        .iter()
        .map(|gate| format!("`{gate}`"))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"# Customize — {name}

This is an owned starter, not generated code you have to throw away. The
smallest useful change should remain easy to locate, run, and verify.

## Source map

| Change | Start here |
|---|---|
| Clinical workflow, routes, and server validation | `app/src/main.rs` |
| Svelte extension workspace | `web/src/routes/+page.svelte` |
| Theme tokens and reusable component classes | `web/src/clinician.css` |
| Rust dependencies and binary settings | `app/Cargo.toml` |
| Svelte dependencies and scripts | `web/package.json` |
| Safe example records | `{fixture}` |
| Browser journey and quality rubric | `artifact-quality.json` |
| Executable browser test | `web/tests/owned-app.mjs` |
| Pack identity, profile, and required gates | `pack.hcl` |
| Production limitations and release evidence | `docs/COMPLIANCE.md` |

Runtime profile: **{profile}**. Current workflow:

{features}

## Make the next change

1. Describe one observable user outcome, such as “staff can filter the queue
   by overdue status.” Avoid starting with a technology choice.
2. Add or adjust synthetic examples in `{fixture}`. Never paste real patient
   information into a fixture, prompt, screenshot, or test.
3. Implement server behavior in `app/src/main.rs` and screen behavior in
   `web/src/routes/+page.svelte`. Keep authorization checks and safety
   disclosures on every new route.
4. Update the contract in `artifact-quality.json` and the executable test in
   `web/tests/owned-app.mjs`. The test must prove the user outcome in a browser.
5. Run `cd app && cargo fmt --check && cargo test`. Then run
   `cd web && npm ci && npm run check && npm run build` from the repository
   root. Build and start the Docker image. Keep it running while you run
   `cd web && npm run test:journey` from another terminal.

## Controls that must survive customization

This pack declares: {gates}. A source edit is not permission to claim a
control is production-ready. Keep the known limitations in
`docs/COMPLIANCE.md` until real infrastructure supplies and verifies them.

## Export or share the next version

- Commit the whole repository so source, fixture, contract, and evidence move
  together.
- Deploy with one of the manifests documented in `docs/RUNBOOK.md`.
- Run `node scripts/reimport.mjs` to import the exact customized bundle as
  your next private synthetic starter.

Before real patient use, replace process-local state and demo credentials,
configure durable audit/storage/backups, enforce workload identity and egress,
and repeat the gate review under the intended BAA boundary.
"#,
        name = app.name,
        profile = pack.profile,
    )
}

// ---------- clean-room design system: owned by the exported project ----------

/// Semantic tokens and a deliberately small component vocabulary shared by
/// exported apps. This is independently authored project code: no supplied
/// archive, Catalyst, Tailwind Plus, font, or demo asset is redistributed.
fn clinician_design_system_css() -> &'static str {
    r#"/* Project-owned warm clinician design system. Customize tokens first. */
:root {
  --hc-canvas: #fbf6f3;
  --hc-surface: #fffdfc;
  --hc-ink: #2c2528;
  --hc-muted: #75696d;
  --hc-line: #e7dadd;
  --hc-brand: #9f3d5f;
  --hc-brand-strong: #762844;
  --hc-focus: #d76b8e;
  --hc-success: #287052;
  --hc-success-bg: #edf7f1;
  --hc-warning: #8b571e;
  --hc-warning-bg: #fff5df;
  --hc-danger: #a43c3c;
  --hc-danger-bg: #fff0ee;
  --hc-radius-control: 12px;
  --hc-radius-card: 18px;
  --hc-shadow-card: 0 12px 32px rgb(70 35 46 / 8%);
  --hc-target: 44px;
}

.hc-page { background: var(--hc-canvas); color: var(--hc-ink); font: 16px/1.5 ui-rounded, system-ui, sans-serif; }
.hc-shell { width: min(72rem, calc(100% - 2rem)); margin-inline: auto; }
.hc-card { padding: 1.25rem; background: var(--hc-surface); border: 1px solid var(--hc-line); border-radius: var(--hc-radius-card); box-shadow: var(--hc-shadow-card); }
.hc-stack { display: grid; gap: 1rem; }
.hc-actions { display: flex; flex-wrap: wrap; align-items: center; gap: .75rem; }
.hc-button { min-height: var(--hc-target); display: inline-flex; align-items: center; justify-content: center; padding: .6rem 1rem; border: 1px solid var(--hc-line); border-radius: var(--hc-radius-control); background: var(--hc-surface); color: var(--hc-ink); font: inherit; font-weight: 700; cursor: pointer; }
.hc-button--primary { border-color: var(--hc-brand); background: var(--hc-brand); color: white; }
.hc-button--primary:hover { background: var(--hc-brand-strong); }
.hc-field { min-height: var(--hc-target); width: 100%; padding: .65rem .8rem; border: 1px solid var(--hc-line); border-radius: var(--hc-radius-control); background: var(--hc-surface); color: var(--hc-ink); font: inherit; }
.hc-label { display: grid; gap: .35rem; font-weight: 700; }
.hc-help { color: var(--hc-muted); font-size: .875rem; font-weight: 400; }
.hc-notice { padding: 1rem; border: 1px solid var(--hc-line); border-left: .35rem solid var(--hc-brand); border-radius: var(--hc-radius-control); background: var(--hc-surface); }
.hc-notice--warning { border-left-color: var(--hc-warning); background: var(--hc-warning-bg); color: var(--hc-warning); }
.hc-badge { display: inline-flex; align-items: center; min-height: 1.75rem; padding: .2rem .6rem; border-radius: 999px; background: var(--hc-success-bg); color: var(--hc-success); font-size: .75rem; font-weight: 800; }
.hc-button:focus-visible, .hc-field:focus-visible, .hc-link:focus-visible { outline: 3px solid color-mix(in srgb, var(--hc-focus) 40%, transparent); outline-offset: 2px; }
@media (prefers-reduced-motion: reduce) { *, *::before, *::after { scroll-behavior: auto !important; transition-duration: .01ms !important; animation-duration: .01ms !important; } }
"#
}

fn design_system_md() -> String {
    r##"# Project-owned clinician design system

`app/assets/clinician.css` belongs to this exported project. It has no runtime
dependency, external font, supplied-archive code, Catalyst code, or Tailwind
Plus asset. Change the `--hc-*` tokens to make the app yours while preserving
the accessible component behavior below.

The Svelte layout already imports this file. Change its tokens or component
classes, then run `npm run check` and `npm run build` from `web/`.

## Components

```html
<section class="hc-card hc-stack">
  <span class="hc-badge">Synthetic learning environment</span>
  <label class="hc-label">Patient-visible label
    <span class="hc-help">Explain what happens with this value.</span>
    <input class="hc-field" name="example" required>
  </label>
  <aside class="hc-notice hc-notice--warning" role="note">Not monitored for emergencies.</aside>
  <div class="hc-actions">
    <button class="hc-button hc-button--primary">Save for review</button>
    <a class="hc-button hc-link" href="/">Cancel</a>
  </div>
</section>
```

Use semantic HTML and visible labels first; classes supply presentation, not
meaning. Interactive controls retain a 44-pixel minimum target, visible focus,
and reduced-motion behavior. Do not encode clinical status by color alone.
"##
    .to_string()
}

// ---------- COMPLIANCE.md: gate report + attestation + audit trail ----------

fn compliance_md(
    app: &AppRecord,
    report: &GateReport,
    provenance: ReportProvenance,
    audit: &[&AuditEvent],
) -> String {
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

    let heading = match provenance {
        ReportProvenance::Frozen => "frozen at promotion",
        ReportProvenance::Rerun => "re-run at export",
    };
    md.push_str(&format!(
        "## Gate report ({heading}, app v{})\n\n{} checks passed — {}\n\n",
        report.app_version,
        report.summary(),
        if report.green {
            "green"
        } else {
            "NOT green: promotion is locked until every check passes"
        }
    ));
    if provenance == ReportProvenance::Frozen {
        md.push_str(
            "This is the attestation-time report, embedded verbatim at release — the \
             evidence that admitted the app, not a re-run.\n\n",
        );
    }
    // Dual-register table (P1): plain-language check + the HIPAA citation it
    // implements; basis says whether the verdict was inspected or claimed.
    md.push_str("| gate | check | HIPAA citation | basis | verdict |\n|---|---|---|---|---|\n");
    for result in &report.results {
        let verdict = match &result.outcome {
            GateStatus::Pass => "pass".to_string(),
            GateStatus::Stubbed { reason } => format!("STUBBED — {}", md_cell(reason)),
            GateStatus::Fail { reason, fixable } => format!(
                "FAIL — {}{}",
                md_cell(reason),
                if *fixable { " (one-click fixable)" } else { "" }
            ),
        };
        let basis = match result.basis {
            gates::Basis::Control => "control (self-reported)",
            gates::Basis::Evidence => "evidence (source inspected)",
        };
        md.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            result.id,
            md_cell(&result.title),
            result.citation.as_deref().unwrap_or("— (pack-defined)"),
            basis,
            verdict
        ));
    }

    md.push_str("\n## Attestation\n\n");
    match (&app.stage, &app.attestation) {
        (Stage::Live, Some(att)) => {
            let principal = att
                .principal
                .as_deref()
                .map(|p| format!(" (authenticated principal `{p}`)"))
                .unwrap_or_default();
            md.push_str(&format!(
                "- co-signed by: **{}**{principal}\n- gate summary at release: **{}**\n",
                att.cosigner, att.gate_summary
            ));
            if let Some(digest) = &att.report_digest {
                md.push_str(&format!(
                    "- gate report digest: `{digest}` — sha256 over the frozen report's \
                     canonical JSON; the co-sign binds exactly this evidence (#10)\n"
                ));
            }
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
        // The ejected bundle is the doctor's own record, so sensitive
        // values render as their own plaintext here — the tenant side of
        // the HMAC boundary (#8, decision 0004). The platform-wide export
        // keeps the hmac-sha256: form.
        let mut detail = event.detail.clone();
        for (key, value) in event.tenant_sensitive() {
            detail.push_str(&format!(" — {key}: {value:?}"));
        }
        md.push_str(&format!(
            "| {} | {} | {} | `{}` | {} |\n",
            event.seq,
            utc(event.at),
            md_cell(&event.actor),
            event.action,
            md_cell(&detail)
        ));
    }
    md.push_str("\n## Known limitations and production responsibilities\n\n");
    md.push_str("This exported scaffold is proven against synthetic data. Any control marked STUBBED in the gate report remains a production blocker: configure authenticated user access, encryption at rest, durable audit retention, approved runtime egress, backups, and incident response before handling real patient information.\n");
    md
}

// ---------- deploy manifests ----------

fn dockerfile(
    app: &AppRecord,
    scaffold: Option<&'static [crate::packs::PackSourceFile]>,
) -> String {
    if let Some(sources) = scaffold {
        // The scaffold crate names its binary `app` ([[bin]] in its
        // Cargo.toml) precisely so this manifest never depends on a package
        // name. Layout mirrors the bundle: server/ crate + synthetic/ seed,
        // whose path is read off the embedded table rather than assumed.
        let seed = sources
            .iter()
            .map(|(path, _)| *path)
            .find(|path| path.starts_with("synthetic/"))
            .unwrap_or("synthetic/");
        return format!(
            "# {} — real app source: this pack's runnable scaffold (issue #5).\n\
             # Builds Svelte and Rust, then serves both from one origin.\n\
             FROM node:22-alpine AS web-build\n\
             WORKDIR /src/web\n\
             COPY web/package.json web/package-lock.json ./\n\
             RUN npm ci --ignore-scripts --no-audit --no-fund\n\
             COPY web ./\n\
             RUN npm run build\n\
             \n\
             FROM rust:1-alpine AS server-build\n\
             RUN apk add --no-cache musl-dev\n\
             WORKDIR /srv\n\
             COPY synthetic ./synthetic\n\
             COPY server ./server\n\
             RUN cargo build --release --locked --manifest-path server/Cargo.toml\n\
             \n\
             FROM nginxinc/nginx-unprivileged:1.29-alpine\n\
             COPY --from=web-build /src/web/build /usr/share/nginx/html/workspace\n\
             COPY --from=server-build --chmod=0555 /srv/server/target/release/app /usr/local/bin/app\n\
             COPY synthetic /srv/synthetic\n\
             COPY config/nginx.conf /etc/nginx/nginx.conf\n\
             COPY --chmod=0555 config/start.sh /usr/local/bin/start.sh\n\
             ENV APP_BIND=127.0.0.1:8081\n\
             ENV SYNTHETIC_DATA=/srv/{seed}\n\
             EXPOSE 8080\n\
             CMD [\"/usr/local/bin/start.sh\"]\n",
            app.name
        );
    }
    format!(
        "# {} — placeholder runtime (see README, \"Honest caveat\").\n\
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

fn clinical_entry_path(scaffold: Option<&[crate::packs::PackSourceFile]>) -> String {
    let path = scaffold
        .and_then(|files| {
            files
                .iter()
                .find(|(path, _)| *path == "artifact-quality.json")
        })
        .and_then(|(_, content)| serde_json::from_str::<serde_json::Value>(content).ok())
        .and_then(|contract| {
            contract
                .pointer("/quality/job/journeys/0/steps")?
                .as_array()?
                .iter()
                .find(|step| step.get("do").and_then(serde_json::Value::as_str) == Some("goto"))?
                .get("path")?
                .as_str()
                .map(str::to_string)
        });
    path.filter(|path| {
        path.starts_with('/')
            && !path.starts_with("//")
            && path
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
    })
    .unwrap_or_else(|| "/".to_string())
}

fn nginx_config(clinical_entry: &str) -> String {
    let clinical_root = if clinical_entry == "/" {
        String::new()
    } else {
        format!("    location = / {{\n      return 302 {clinical_entry};\n    }}\n\n")
    };
    r#"pid /tmp/nginx.pid;
error_log /dev/stderr info;

events {}

http {
  include /etc/nginx/mime.types;
  default_type application/octet-stream;
  absolute_redirect off;
  access_log /dev/stdout;
  client_body_temp_path /tmp/client-body;
  proxy_temp_path /tmp/proxy;
  client_max_body_size 26m;

  server {
    listen 8080;
    server_name _;

    # CLINICAL_ROOT
    location = /workspace {
      return 308 /workspace/;
    }

    location ^~ /workspace/_app/ {
      root /usr/share/nginx/html;
      try_files $uri =404;
      expires 1y;
      add_header Cache-Control "public, immutable";
    }

    location /workspace/ {
      root /usr/share/nginx/html;
      try_files $uri $uri/ /workspace/index.html;
    }

    location / {
      proxy_pass http://127.0.0.1:8081;
      proxy_http_version 1.1;
      proxy_buffering off;
      proxy_request_buffering off;
      proxy_read_timeout 3600s;
      # Preserve the public port so Rust's Origin/Host comparison remains
      # correct when the exported container is published on a random port.
      proxy_set_header Host $http_host;
      proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
      proxy_set_header X-Forwarded-Proto $scheme;
    }
  }
}
"#
    .replace("    # CLINICAL_ROOT\n", &clinical_root)
}

fn runtime_start_script() -> String {
    r#"#!/bin/sh
set -eu

app &
app_pid=$!
nginx -g 'daemon off;' &
nginx_pid=$!

shutdown() {
  trap - INT TERM
  kill -TERM "$app_pid" "$nginx_pid" 2>/dev/null || true
  wait "$app_pid" "$nginx_pid" 2>/dev/null || true
  exit 0
}
trap shutdown INT TERM

while kill -0 "$app_pid" 2>/dev/null && kill -0 "$nginx_pid" 2>/dev/null; do
  sleep 1
done
kill -TERM "$app_pid" "$nginx_pid" 2>/dev/null || true
wait "$app_pid" "$nginx_pid" 2>/dev/null || true
exit 1
"#
    .to_string()
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
/// are gates. Parses only through the untrusted owned-pack parser.
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
         // Practice-owned metadata. This is not a trusted registry signature.\n\n\
         pack \"{id}-template\" {{\n\
         \x20 name        = \"{name}\"\n\
         \x20 description = \"{description}\"\n\
         \x20 profile     = \"{profile}\"\n\
         \x20 tier        = {tier}\n\
         \x20 wave        = {wave}\n\
         \x20 signed_by   = \"untrusted-practice-export\"\n\
         \x20 based_on    = \"{base_pack}\"\n\n\
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
        base_pack = hcl_str(&pack.id),
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

    /// The dev registry's meridian clinician — the co-signing principal.
    fn dr_osei() -> crate::identity::Principal {
        crate::identity::Registry::dev_default()
            .by_token("dev-token-osei")
            .unwrap()
            .clone()
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
            external_calls: vec![],
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

        assert!(
            packs::parse_pack(&hcl).is_err(),
            "owned metadata is not trusted"
        );
        let template = packs::parse_owned_pack(&hcl).expect("derived owned pack must parse");
        assert_eq!(template.id, "post-op-tracker-template");
        assert_eq!(template.signed_by, "untrusted-practice-export");
        assert_eq!(template.based_on.as_deref(), Some(pack.id.as_str()));
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
    fn converted_pack_bundle_ships_real_scaffold_source_and_drops_the_caveat() {
        let pack = post_op_pack();
        assert_eq!(pack.scaffold_path.as_deref(), Some("scaffold"));
        let app = sample_app(&pack);
        let bundle = bundle(&app, &pack, &[]);

        // The scaffold's source tree lands under server/, byte-identical to
        // the compile-time embedded packs/post-op-monitor/scaffold/.
        let main_rs = &bundle.files["server/src/main.rs"];
        assert!(main_rs.contains("PAIN_ESCALATION_THRESHOLD"));
        assert!(main_rs.contains(".route(\"/api/checkins\", post(api_checkin))"));
        assert!(main_rs.contains("Idempotency-Key"));
        assert!(bundle.files["server/Cargo.toml"].contains("post-op-monitor-scaffold"));
        assert!(bundle.files["server/Cargo.lock"].contains("name = \"axum\""));
        // The synthetic seed rides along where the app's loader expects it.
        assert!(bundle.files["synthetic/post-op-demo.json"]
            .contains("SYNTHETIC DATA — generated, not derived from any real person"));

        // The runbook stops apologizing: real source, no placeholder caveat.
        let runbook = &bundle.files["README.md"];
        assert!(!runbook.contains("scaffold placeholder"), "{runbook}");
        assert!(runbook.contains("The app source is real"));
        assert!(runbook.contains("cd server"));
        assert!(runbook.contains("APP_BIND=127.0.0.1:8080 cargo run"));

        // And the Dockerfile builds the real crate instead of the stub.
        let dockerfile = &bundle.files["Dockerfile"];
        assert!(dockerfile.contains("FROM node:22-alpine AS web-build"));
        assert!(dockerfile.contains("FROM rust:1-alpine AS server-build"));
        assert!(dockerfile.contains("FROM nginxinc/nginx-unprivileged:1.29-alpine"));
        assert!(dockerfile.contains("cargo build --release --locked"));
        assert!(dockerfile
            .contains("COPY --from=web-build /src/web/build /usr/share/nginx/html/workspace"));
        assert!(dockerfile.contains("SYNTHETIC_DATA=/srv/synthetic/post-op-demo.json"));
        assert!(!dockerfile.contains("python3"));
        assert!(bundle.files["config/nginx.conf"].contains("include /etc/nginx/mime.types"));
        assert!(bundle.files["config/nginx.conf"].contains("absolute_redirect off"));
        assert!(bundle.files["config/nginx.conf"].contains("proxy_set_header Host $http_host"));
        assert!(bundle.files["config/nginx.conf"].contains("return 302 /login"));
        let page = &bundle.files["web/src/routes/+page.svelte"];
        assert!(page.contains("PostOpCheckIn"));
        let checkin = &bundle.files["web/src/lib/PostOpCheckIn.svelte"];
        assert!(checkin.contains("import treatment from './treatment.json'"));
        assert!(checkin.contains("treatment.refinement.presentation === 'context-first'"));
        assert!(!checkin.contains("{@html"));
        assert!(checkin.contains("fetch('/api/checkins'"));
        assert!(checkin.contains("/login?next=/workspace/"));
        assert!(checkin.contains("Queued in the synthetic practice inbox"));
        assert!(checkin.contains("No client-side guess"));
        let treatment: serde_json::Value =
            serde_json::from_str(&bundle.files["web/src/lib/treatment.json"]).unwrap();
        assert_eq!(treatment["materializer"], "rust-convention-v2");
        assert!(
            bundle.files["artifact-quality.json"].contains("svelte-pain-eight-reaches-rust-inbox")
        );
        assert!(bundle.files.contains_key("web/tests/owned-app.mjs"));
        assert!(bundle.files["web/tests/owned-app.mjs"].contains("const postOp = true"));
        assert!(bundle.files["web/tests/owned-app.mjs"].contains("Pain 8"));
        assert!(bundle.files["web/package.json"].contains("\"test:journey\""));
        assert!(bundle.files["web/package.json"].contains("\"playwright\": \"1.61.1\""));
        assert!(bundle.files["README.md"].contains("npm run test:journey"));
        assert!(bundle.files[".gitignore"].contains("web/test-results/"));
        assert!(bundle.files[".dockerignore"].contains("web/node_modules"));
        assert!(bundle.files[".dockerignore"].contains("web/test-results"));
        assert!(bundle.files.contains_key("scripts/reimport.mjs"));
        validate_owned_bundle(&bundle.files).unwrap();

        let mut traversal = bundle.files.clone();
        traversal.insert("../escape".into(), "no".into());
        assert!(validate_owned_bundle(&traversal).is_err());
        let mut collision = bundle.files.clone();
        collision.insert("readme.md".into(), "collision".into());
        assert!(validate_owned_bundle(&collision).is_err());
        let mut generated = bundle.files.clone();
        generated.insert("web/node_modules/rogue.js".into(), "no".into());
        assert!(validate_owned_bundle(&generated).is_err());
        let mut incomplete = bundle.files.clone();
        incomplete.remove("web/tests/owned-app.mjs");
        assert!(validate_owned_bundle(&incomplete).is_err());
    }

    #[test]
    fn every_built_in_pack_bundle_carries_real_source_and_quality_contract() {
        let pack = packs::builtin_packs()
            .into_iter()
            .find(|p| p.id == "hypertension-tracker")
            .expect("hypertension-tracker is a built-in pack");
        assert_eq!(pack.scaffold_path.as_deref(), Some("scaffold"));
        let mut app = sample_app(&pack);
        app.pack = pack.id.clone();
        let bundle = bundle(&app, &pack, &[]);

        assert!(bundle.files.contains_key("server/src/main.rs"));
        assert!(bundle.files.contains_key("web/src/routes/+page.svelte"));
        assert!(bundle.files.contains_key("artifact-quality.json"));
        assert!(bundle.files.contains_key("web/tests/owned-app.mjs"));
        assert!(bundle.files["web/tests/owned-app.mjs"].contains("const postOp = false"));
        let runbook = &bundle.files["README.md"];
        assert!(!runbook.contains("scaffold placeholder"));
        assert!(!runbook.contains("photo upload stub"));
        let customize = &bundle.files["README.md"];
        assert!(customize.contains("## Source map"));
        assert!(customize.contains("synthetic/htn-demo.json"));
        assert!(customize.contains("## Make the next change"));
        assert!(customize.contains("## Export or share the next version"));
        assert!(customize.contains("web/src/clinician.css"));
        assert!(customize.contains("Theme tokens and reusable component classes"));
        let css = &bundle.files["web/src/clinician.css"];
        assert!(css.contains("--hc-brand:"), "semantic brand token missing");
        assert!(css.contains("--hc-target: 44px"), "minimum target missing");
        assert!(css.contains(":focus-visible"), "visible focus missing");
        assert!(
            css.contains("prefers-reduced-motion: reduce"),
            "reduced-motion behavior missing"
        );
        assert!(css.contains(".hc-card"), "card component missing");
        assert!(css.contains(".hc-notice"), "notice component missing");
        assert!(!css.contains("Catalyst"));
        assert!(!css.contains("Tailwind"));
        assert!(!css.contains("Shakti"));
        let design_docs = &bundle.files["README.md"];
        assert!(design_docs.contains("Project-owned clinician design system"));
        assert!(design_docs.contains("web/src/clinician.css"));
        assert!(design_docs.contains("The Svelte layout already imports this file"));
        assert!(!design_docs.contains("include_str!(\"../assets/clinician.css\")"));
        assert!(!bundle.files.keys().any(|path| path.starts_with("docs/")));
        assert!(bundle.files["Dockerfile"].contains("FROM node:22-alpine AS web-build"));
        assert!(bundle.files.contains_key("config/nginx.conf"));
        assert!(bundle.files.contains_key("config/start.sh"));
    }

    #[test]
    fn owned_bundle_has_svelte_rust_mcp_three_diagrams_and_one_readme() {
        let pack = post_op_pack();
        let bundle = bundle(&sample_app(&pack), &pack, &[]);

        assert!(bundle.files["web/package.json"].contains("\"svelte\": \"5.56.4\""));
        assert!(bundle.files.contains_key("web/package-lock.json"));
        let lock: serde_json::Value =
            serde_json::from_str(&bundle.files["web/package-lock.json"]).unwrap();
        assert!(lock["packages"]
            .as_object()
            .unwrap()
            .keys()
            .all(|path| path.is_empty() || path.starts_with("node_modules/")));
        assert!(bundle.files["web/svelte.config.js"].contains("adapter-static"));
        assert!(bundle.files["web/svelte.config.js"].contains("base: '/workspace'"));
        assert!(bundle.files["web/src/routes/+page.svelte"].contains("fetch('/health',"));
        assert!(bundle.files["web/src/routes/+page.svelte"].contains("Rust service connected"));
        assert!(bundle.files["web/src/routes/+page.svelte"].contains("$state"));
        assert!(bundle.files["README.md"].contains("cd server && cargo fmt --check"));
        assert!(bundle.files["README.md"].contains("Gemma as its only application model"));
        assert!(!bundle.files["README.md"].contains("LangChain"));
        assert!(!bundle.files["README.md"].contains("Deep Agents"));
        assert!(!bundle.files["README.md"].contains("Open SWE"));
        assert!(bundle.files.contains_key("server/Cargo.toml"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&bundle.files[".mcp.json"]).unwrap()
                ["mcpServers"]["svelte"]["url"],
            "https://mcp.svelte.dev/mcp"
        );

        let diagrams = bundle
            .files
            .iter()
            .filter(|(path, _)| path.ends_with(".tldr"))
            .collect::<Vec<_>>();
        assert_eq!(diagrams.len(), 3);
        for (_, raw) in diagrams {
            let diagram: serde_json::Value = serde_json::from_str(raw).unwrap();
            assert_eq!(diagram["tldrawFileFormatVersion"], 1);
            assert!(diagram["records"].as_array().unwrap().len() >= 6);
        }

        let prose = bundle
            .files
            .keys()
            .filter(|path| path.ends_with(".md") || path.ends_with(".mdx"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(prose, vec!["README.md"]);
        assert!(!bundle.files.keys().any(|path| path.starts_with("docs/")));
    }

    #[test]
    fn all_seventeen_exports_use_no_secondary_model_runtime() {
        let packs = packs::builtin_packs();
        assert_eq!(packs.len(), 17);
        for pack in &packs {
            let bundle = bundle(&sample_app(pack), pack, &[]);
            let prose = bundle
                .files
                .keys()
                .filter(|path| path.ends_with(".md") || path.ends_with(".mdx"))
                .collect::<Vec<_>>();
            assert_eq!(prose, vec![&"README.md".to_string()], "{}", pack.id);
            assert_eq!(
                bundle
                    .files
                    .keys()
                    .filter(|path| path.ends_with(".tldr"))
                    .count(),
                3,
                "{}",
                pack.id
            );
            let all = bundle.files.values().cloned().collect::<String>();
            assert!(!bundle.files.contains_key("server/src/local_media.rs"));
            assert!(!bundle
                .files
                .contains_key("web/src/lib/LocalMediaInput.svelte"));
            for forbidden in [
                "LFM2",
                "LIQUID_AUDIO_URL",
                "LIQUID_VISION_URL",
                "/api/local-media/",
                "Optional same-host Liquid",
            ] {
                assert!(!all.contains(forbidden), "{} leaked {forbidden}", pack.id);
            }
        }
    }

    #[test]
    fn sandbox_bundle_is_draft_with_no_attestation_and_stub_job() {
        let pack = post_op_pack();
        let app = sample_app(&pack);
        let bundle = bundle(&app, &pack, &[]);

        let compliance = &bundle.files["README.md"];
        assert!(compliance.contains("draft — not released"));
        assert!(compliance.contains("omitted by design"));
        assert!(!compliance.contains("co-signed by:"));
        // Draft bundles keep the live re-run (F3 applies to releases only).
        assert!(compliance.contains("Gate report (re-run at export"));
        assert!(bundle.files["nomad/job.nomad.hcl"].contains("no live allocation yet"));
        assert!(bundle.unpack.contains("python3"));
    }

    #[test]
    fn live_bundle_embeds_the_frozen_attestation_report_and_rendered_job() {
        let pack = post_op_pack();
        let mut app = sample_app(&pack);
        let mut report = gates::preflight(&app, &pack.gates);
        assert_eq!(report.stubbed, 1);
        for result in &mut report.results {
            if matches!(result.outcome, GateStatus::Stubbed { .. }) {
                result.outcome = GateStatus::Pass;
            }
        }
        report.passed += report.stubbed;
        report.stubbed = 0;
        report.green = true;
        deploy::promote(
            &mut app,
            &report,
            &dr_osei(),
            Some("Dr. A. Osei"),
            "a-0001".to_string(),
            false,
        )
        .expect("promotion succeeds on a green report");

        // F3: the attestation carries the admitting report verbatim.
        assert!(app.attestation.as_ref().unwrap().report.is_some());

        let bundle = bundle(&app, &pack, &[]);
        let compliance = &bundle.files["README.md"];
        assert!(compliance.contains("Status: **released**"));
        assert!(compliance.contains("**Dr. A. Osei**"));
        // #10: the record renders the cryptographic act — principal id and
        // the sha256 digest of the frozen report the co-sign binds.
        assert!(compliance.contains("(authenticated principal `dr-osei`)"));
        let att = app.attestation.as_ref().unwrap();
        let digest = att.report_digest.as_deref().unwrap();
        assert!(compliance.contains(digest), "{compliance}");
        assert_eq!(
            digest,
            gates::report_digest(att.report.as_ref().unwrap()),
            "digest verifies against the frozen report"
        );
        assert!(compliance.contains("**6/6**"));
        // The released record embeds the frozen attestation-time report —
        // never a re-run over the (legitimately tenant-backed) live view.
        assert!(compliance.contains("Gate report (frozen at promotion"));
        assert!(compliance.contains("embedded verbatim at release"));
        assert!(!compliance.contains("re-run at export"));
        assert!(compliance.contains("6/6 checks passed — green"));
        // Dual-register vocabulary (P1): citations next to plain language.
        assert!(compliance.contains("45 CFR §164.312(b)"));
        assert!(compliance.contains("evidence (source inspected)"));
        assert!(app.allocation.is_some());
        assert!(bundle.files["README.md"].contains("knee replacement patients"));
    }

    #[test]
    fn frozen_report_survives_even_though_the_live_app_reads_tenant_data() {
        // The F3 rationale made concrete: after promotion the app record is
        // tenant-wired, so a raw re-run would fail synthetic-only forever.
        // The frozen report keeps the promotion-time truth instead.
        let pack = post_op_pack();
        let mut app = sample_app(&pack);
        let mut report = gates::preflight(&app, &pack.gates);
        for result in &mut report.results {
            if matches!(result.outcome, GateStatus::Stubbed { .. }) {
                result.outcome = GateStatus::Pass;
            }
        }
        report.passed += report.stubbed;
        report.stubbed = 0;
        report.green = true;
        deploy::promote(
            &mut app,
            &report,
            &dr_osei(),
            Some("Dr. A. Osei"),
            "a-0001".to_string(),
            false,
        )
        .unwrap();
        assert!(matches!(app.data_source, DataSource::Tenant(_)));

        let live_rerun = gates::preflight(&app, &pack.gates);
        assert!(!live_rerun.green, "raw re-run fails synthetic-only");

        let (frozen, provenance) = preflight_report(&app, &pack);
        assert_eq!(provenance, ReportProvenance::Frozen);
        assert!(frozen.green, "the frozen admitting report is the record");
        assert_eq!(frozen.summary(), "6/6");
    }
}
