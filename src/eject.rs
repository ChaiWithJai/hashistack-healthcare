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
use crate::packs::{InputCapability, PackManifest};
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
    let (report, provenance) = preflight_report(app, pack);
    let has_audio = pack
        .input_capabilities
        .contains(&InputCapability::LocalAudioTranscription);
    let has_image = pack
        .input_capabilities
        .contains(&InputCapability::LocalImageDescription);
    let has_local_media = has_audio || has_image;
    // Packs converted to the full spec (#5) ship a runnable scaffold whose
    // sources are compile-time embedded next to the manifests (packs.rs) —
    // the same no-drift guarantee PACK_SOURCES gives. `scaffold_path` on the
    // signed manifest is the opt-in; the embedded table is the content.
    let scaffold = pack
        .scaffold_path
        .as_deref()
        .and_then(|_| crate::packs::scaffold_sources(&pack.id));
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
            let content = if dest == "server/src/main.rs" && has_local_media {
                content.replace(
                    "#[path = \"../../../visit-notes/scaffold/src/local_media.rs\"]\n",
                    "",
                )
            } else {
                (*content).to_string()
            };
            files.insert(dest, content);
        }
        files.insert(
            "web/src/clinician.css".to_string(),
            clinician_design_system_css().to_string(),
        );
    }
    if has_local_media {
        files.insert(
            "server/src/local_media.rs".to_string(),
            crate::packs::local_media_source().to_string(),
        );
    }
    files.insert("web/package.json".to_string(), svelte_package_json());
    files.insert("web/svelte.config.js".to_string(), svelte_config());
    files.insert(
        "web/vite.config.ts".to_string(),
        svelte_vite_config(has_local_media),
    );
    files.insert("web/tsconfig.json".to_string(), svelte_tsconfig());
    files.insert("web/src/app.html".to_string(), svelte_app_html());
    files.insert("web/src/app.d.ts".to_string(), svelte_app_types());
    files.insert("web/src/routes/+layout.svelte".to_string(), svelte_layout());
    files.insert(
        "web/src/routes/+page.svelte".to_string(),
        svelte_page(app, pack),
    );
    if has_local_media {
        files.insert(
            "web/src/lib/LocalMediaInput.svelte".to_string(),
            local_media_input_component(),
        );
    }
    files.insert(".mcp.json".to_string(), svelte_mcp_config());
    files.insert(
        "diagrams/system-architecture.tldr".to_string(),
        tldraw_diagram(
            "System architecture",
            &[
                "Svelte 5 web",
                "Rust and Axum server",
                "Synthetic data",
                "Owned deployment",
            ]
            .into_iter()
            .chain(has_local_media.then_some("Optional same-host Liquid (local dev only)"))
            .collect::<Vec<_>>(),
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
                "Agent worker",
                "Gate",
                "Runtime",
            ]
            .into_iter()
            .chain(has_local_media.then_some("Optional same-host Liquid (local dev only)"))
            .collect::<Vec<_>>(),
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
            .replace("docs/DESIGN_SYSTEM.md", "the design system section below")
            .replace("docs/COMPLIANCE.md", "the compliance section below")
            .replace("docs/RUNBOOK.md", "the run instructions above"),
    );
    readme.push_str("\n\n");
    readme
        .push_str(&design_system_md().replace("app/assets/clinician.css", "web/src/clinician.css"));
    readme.push_str("\n\n");
    readme.push_str(&compliance_md(app, &report, provenance, audit));
    readme.push_str("\n\n## Extend with AI\n\nThe `.mcp.json` file connects compatible editors to the official Svelte MCP server. Use it to check Svelte 5 APIs and repair components. If you add an agent, keep it behind the Rust API and use LangChain or LangGraph as an optional worker. Do not give a model production secrets, deployment authority, or patient data.\n");
    if has_local_media {
        readme.push_str(&local_media_readme(has_audio, has_image));
    }
    files.insert("README.md".to_string(), readme);
    files.insert("Dockerfile".to_string(), dockerfile(app, scaffold));
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

    md.push_str("## Repository map\n\n");
    md.push_str(&format!(
        "- `web/` contains the Svelte 5 interface.\n\
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
    "check": "svelte-kit sync && svelte-check --tsconfig ./tsconfig.json"
  },
  "devDependencies": {
    "@sveltejs/adapter-auto": "^7.0.1",
    "@sveltejs/kit": "^2.69.2",
    "@sveltejs/vite-plugin-svelte": "^7.2.0",
    "svelte": "^5.56.4",
    "svelte-check": "^4.4.5",
    "typescript": "^5.9.3",
    "vite": "^8.1.4"
  }
}
"#
    .to_string()
}

fn svelte_config() -> String {
    "import adapter from '@sveltejs/adapter-auto';\n\nexport default { kit: { adapter: adapter() } };\n".to_string()
}

fn svelte_vite_config(local_media: bool) -> String {
    if local_media {
        "import { sveltekit } from '@sveltejs/kit/vite';\nimport { defineConfig } from 'vite';\n\nexport default defineConfig({\n  plugins: [sveltekit()],\n  server: { proxy: { '/api/local-media': 'http://127.0.0.1:8080' } }\n});\n".to_string()
    } else {
        "import { sveltekit } from '@sveltejs/kit/vite';\nimport { defineConfig } from 'vite';\n\nexport default defineConfig({ plugins: [sveltekit()] });\n".to_string()
    }
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

fn svelte_page(app: &AppRecord, pack: &PackManifest) -> String {
    let features = app
        .features
        .iter()
        .map(|feature| format!("    {{ label: {:?}, done: true }}", feature))
        .collect::<Vec<_>>()
        .join(",\n");
    let local_media = if pack
        .input_capabilities
        .contains(&InputCapability::LocalAudioTranscription)
    {
        (
            "  import LocalMediaInput from '../lib/LocalMediaInput.svelte';\n",
            "\n    <LocalMediaInput kind=\"audio\" onaccept={(text) => { note = text; addItem(); }} />",
        )
    } else if pack
        .input_capabilities
        .contains(&InputCapability::LocalImageDescription)
    {
        (
            "  import LocalMediaInput from '../lib/LocalMediaInput.svelte';\n",
            "\n    <LocalMediaInput kind=\"image\" onaccept={(text) => { note = text; addItem(); }} />",
        )
    } else {
        ("", "")
    };
    format!(
        r#"<script lang="ts">
{media_import}
  let items = $state([
{features}
  ]);
  let note = $state('');

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
    </header>
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
{media_control}
      <aside class="hc-notice hc-notice--warning" role="note">This starter is not monitored for emergencies or approved for clinical care.</aside>
    </section>
  </div>
</main>
"#,
        name = app.name,
        description = pack.description,
        media_import = local_media.0,
        media_control = local_media.1,
    )
}

fn local_media_input_component() -> String {
    r#"<script lang="ts">
  let { kind, onaccept }: { kind: 'audio' | 'image'; onaccept: (text: string) => void } = $props();
  let file: File | undefined = $state();
  let input: HTMLInputElement;
  let acknowledged = $state(false);
  let observation = $state('');
  let model = $state('');
  let digest = $state('');
  let error = $state('');
  let working = $state(false);
  let controller: AbortController | undefined = $state();
  let isAudio = $derived(kind === 'audio');
  let label = $derived(isAudio ? 'synthetic WAV recording' : 'synthetic PNG image');

  async function analyze() {
    if (!file || !acknowledged || working) return;
    working = true;
    error = '';
    observation = '';
    model = '';
    digest = '';
    controller = new AbortController();
    try {
      const response = await fetch(`/api/local-media/${kind}`, {
        method: 'POST',
        headers: {
          'content-type': file.type || 'application/octet-stream',
          'x-synthetic-workflow': 'true'
        },
        body: file,
        signal: controller.signal
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.message || body.error || 'The local model is unavailable.');
      observation = body.text;
      model = body.model;
      digest = body.sha256;
    } catch (cause) {
      error = cause instanceof DOMException && cause.name === 'AbortError'
        ? 'Stopped waiting for local analysis. The local model may finish before its 30-second limit.'
        : cause instanceof Error ? cause.message : 'The local model is unavailable.';
    } finally {
      working = false;
      controller = undefined;
      file = undefined;
      input.value = '';
    }
  }

  function useObservation() {
    onaccept(observation);
    observation = '';
    model = '';
    digest = '';
  }
</script>

<section class="hc-stack" aria-labelledby="local-media-title">
  <h3 id="local-media-title">Add a local model observation</h3>
  <p class="hc-help">Same-device development only. Your browser sends the {kind} to this local Rust server, which accepts only a loopback Liquid process. It is never sent to the hosted workspace agent. The result is an untrusted draft—verify it.</p>
  <label class="hc-actions"><input type="checkbox" bind:checked={acknowledged} /> I confirm this is synthetic learning media, not patient data.</label>
  <label class="hc-label">Choose a {label}
    <input bind:this={input} class="hc-field" type="file" accept={isAudio ? '.wav,audio/wav' : '.png,image/png'} onchange={(event) => file = event.currentTarget.files?.[0]} />
  </label>
  <div class="hc-actions">
    <button class="hc-button" disabled={!file || !acknowledged || working} onclick={analyze}>{working ? 'Analyzing locally…' : 'Analyze locally'}</button>
    {#if working}<button class="hc-button" onclick={() => controller?.abort()}>Stop waiting</button>{/if}
  </div>
  {#if error}<p class="hc-notice hc-notice--warning" role="alert">{error} The text-only workflow still works.</p>{/if}
  {#if observation}
    <label class="hc-label">Local model observation—verify before use
      <textarea class="hc-field" bind:value={observation} rows="6"></textarea>
    </label>
    <p class="hc-help">Model: {model}. Source digest: {digest}. The Rust server does not retain the raw file or this draft.</p>
    <button class="hc-button hc-button--primary" onclick={useObservation}>Add observation to workflow</button>
  {/if}
</section>
"#.to_string()
}

fn local_media_readme(audio: bool, image: bool) -> String {
    let model = if audio {
        "`LFM2.5-Audio-1.5B` through `LIQUID_AUDIO_URL`"
    } else if image {
        "`LFM2.5-VL-1.6B` through `LIQUID_VISION_URL`"
    } else {
        unreachable!("local media README requires a capability")
    };
    format!(
        "\n\n## Local audio and vision\n\nThis optional local-development path asks {model} for one bounded, non-diagnostic observation. It is not enabled by the Docker or cloud deploy files. Run the Rust server with `cargo run --manifest-path server/Cargo.toml` (port 8080), run the Svelte UI with `cd web && npm install && npm run dev`, and set the model URL to a same-host adapter such as `http://127.0.0.1:8081/infer`. The adapter accepts the raw request body and returns JSON shaped as `{{\"text\":\"…\"}}`. For the post-op pack, sign in at `http://127.0.0.1:8080/login` before using the Vite UI.\n\n“Local” means the browser, Rust server, and Liquid process run on the same device or network namespace. Do not enable this path on Render, Fly, DigitalOcean, or a separate frontend. Rust accepts only `127.0.0.1`, `localhost`, or `[::1]`, never falls back to a hosted model, and does not write or log the raw body. Configure the Liquid adapter for no retention; this repository cannot control another process's logs. Only synthetic examples are allowed. Verify and edit every result. If Liquid is stopped, continue with the text-only workflow.\n"
    )
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
         `app/` is this pack's runnable standalone Rust (axum) crate. It implements\n\
         the workflow described in this repository's README and boots from the\n\
         included `synthetic/` fixture. Pack-specific limitations stay visible in\n\
         the app and `docs/COMPLIANCE.md`; no generic feature is implied. Run it\n\
         directly:\n\n\
         ```bash\n\
         cd app && cargo run    # http://127.0.0.1:8080 — or APP_BIND=host:port\n\
         cargo test             # the scaffold's own contract\n\
         ```\n"
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
         ## Unpack (if you received the raw export JSON)\n\n\
         ```bash\n\
         {unpack}\n\
         ```\n\n\
         ## Run with Docker\n\n\
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
| Workflow, routes, forms, and validation | `app/src/main.rs` |
| Theme tokens and reusable component classes | `app/assets/clinician.css` |
| Design-system integration and component examples | `docs/DESIGN_SYSTEM.md` |
| Dependencies and binary settings | `app/Cargo.toml` |
| Safe example records | `{fixture}` |
| Browser journey and quality rubric | `artifact-quality.json` |
| Pack identity, profile, and required gates | `pack.hcl` |
| Production limitations and release evidence | `docs/COMPLIANCE.md` |

Runtime profile: **{profile}**. Current workflow:

{features}

## Make the next change

1. Describe one observable user outcome, such as “staff can filter the queue
   by overdue status.” Avoid starting with a technology choice.
2. Add or adjust synthetic examples in `{fixture}`. Never paste real patient
   information into a fixture, prompt, screenshot, or test.
3. Implement the behavior in `app/src/main.rs`. Keep authorization checks and
   safety disclosures on every new route.
4. Extend the journey in `artifact-quality.json` so a browser proves the new
   outcome. Update required labels and honesty text when the UI changes.
5. Run `cd app && cargo fmt --check && cargo test`, then boot the app and run
   the browser journey before sharing it.

## Controls that must survive customization

This pack declares: {gates}. A source edit is not permission to claim a
control is production-ready. Keep the known limitations in
`docs/COMPLIANCE.md` until real infrastructure supplies and verifies them.

## Export or share the next version

- Commit the whole repository so source, fixture, contract, and evidence move
  together.
- Deploy with one of the manifests documented in `docs/RUNBOOK.md`.
- Re-import or share `pack.hcl` to use this customized app as the next
  practice-owned starter.

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

This asset is **opt-in**. The pack's existing pages keep their original inline
styles until you wire this file in; exporting it does not silently restyle a
clinical workflow.

## Option A: embed it in an Axum page

Keep the export self-contained by embedding the stylesheet at compile time:

```rust
const CLINICIAN_CSS: &str = include_str!("../assets/clinician.css");

fn page(body: &str) -> String {
    format!(r#"<!doctype html><html lang="en"><meta name="viewport" content="width=device-width"><style>{CLINICIAN_CSS}</style><body class="hc-page"><main class="hc-shell hc-stack">{body}</main></body></html>"#)
}
```

## Option B: serve and link the stylesheet

Add the route to your existing `Router`:

```rust
use axum::http::{header, HeaderValue};

async fn clinician_css() -> ([(header::HeaderName, HeaderValue); 1], &'static str) {
    ([
        (header::CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8")),
    ], include_str!("../assets/clinician.css"))
}

let app = Router::new()
    .route("/assets/clinician.css", get(clinician_css))
    // keep the pack's existing routes below
    .route("/", get(home));
```

Then put this in each generated page's `<head>` and add `class="hc-page"` to
its `<body>`:

```html
<link rel="stylesheet" href="/assets/clinician.css">
```

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
             # Builds server/ and boots it against the bundled synthetic dataset.\n\
             FROM rust:1-alpine AS build\n\
             RUN apk add --no-cache musl-dev\n\
             WORKDIR /srv\n\
             COPY synthetic ./synthetic\n\
             COPY server ./server\n\
             RUN cargo build --release --manifest-path server/Cargo.toml\n\
             \n\
             FROM alpine:3\n\
             COPY --from=build /srv/server/target/release/app /usr/local/bin/app\n\
             COPY synthetic /srv/synthetic\n\
             ENV APP_BIND=0.0.0.0:8080\n\
             ENV SYNTHETIC_DATA=/srv/{seed}\n\
             EXPOSE 8080\n\
             CMD [\"app\"]\n",
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
    let input_capabilities = pack
        .input_capabilities
        .iter()
        .map(|capability| match capability {
            crate::packs::InputCapability::LocalAudioTranscription => "local-audio-transcription",
            crate::packs::InputCapability::LocalImageDescription => "local-image-description",
        });
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
         \x20 # Local media reach carried from the signed source pack.\n\
         \x20 input_capabilities = [\n{input_capabilities}\x20 ]\n\n\
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
        input_capabilities = hcl_list(input_capabilities),
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
        assert_eq!(template.input_capabilities, pack.input_capabilities);
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
        assert!(bundle.files["server/Cargo.toml"].contains("post-op-monitor-scaffold"));
        // The synthetic seed rides along where the app's loader expects it.
        assert!(bundle.files["synthetic/post-op-demo.json"]
            .contains("SYNTHETIC DATA — generated, not derived from any real person"));

        // The runbook stops apologizing: real source, no placeholder caveat.
        let runbook = &bundle.files["README.md"];
        assert!(!runbook.contains("scaffold placeholder"), "{runbook}");
        assert!(runbook.contains("The app source is real"));
        assert!(runbook.contains("cd server && cargo run"));

        // And the Dockerfile builds the real crate instead of the stub.
        let dockerfile = &bundle.files["Dockerfile"];
        assert!(dockerfile.contains("FROM rust:1-alpine AS build"));
        assert!(dockerfile.contains("SYNTHETIC_DATA=/srv/synthetic/post-op-demo.json"));
        assert!(!dockerfile.contains("python3"));
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
        let runbook = &bundle.files["README.md"];
        assert!(!runbook.contains("scaffold placeholder"));
        assert!(!runbook.contains("photo upload stub"));
        let customize = &bundle.files["README.md"];
        assert!(customize.contains("## Source map"));
        assert!(customize.contains("synthetic/htn-demo.json"));
        assert!(customize.contains("## Make the next change"));
        assert!(customize.contains("## Export or share the next version"));
        assert!(customize.contains("web/src/clinician.css"));
        assert!(customize.contains("the design system section below"));
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
        assert!(design_docs.contains("**opt-in**"));
        assert!(design_docs.contains("include_str!(\"../assets/clinician.css\")"));
        assert!(design_docs.contains("/assets/clinician.css"));
        assert!(design_docs.contains("<link rel=\"stylesheet\""));
        assert!(!bundle.files.keys().any(|path| path.starts_with("docs/")));
        assert!(bundle.files["Dockerfile"].contains("FROM rust:1-alpine AS build"));
    }

    #[test]
    fn owned_bundle_has_svelte_rust_mcp_three_diagrams_and_one_readme() {
        let pack = post_op_pack();
        let bundle = bundle(&sample_app(&pack), &pack, &[]);

        assert!(bundle.files["web/package.json"].contains("\"svelte\": \"^5"));
        assert!(bundle.files["web/src/routes/+page.svelte"].contains("$state"));
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
    fn all_seventeen_exports_prove_three_media_opt_ins_and_fourteen_absences() {
        let packs = packs::builtin_packs();
        assert_eq!(packs.len(), 17);
        let mut audio = 0;
        let mut image = 0;
        let mut ordinary = 0;
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
            let has_audio = pack
                .input_capabilities
                .contains(&InputCapability::LocalAudioTranscription);
            let has_image = pack
                .input_capabilities
                .contains(&InputCapability::LocalImageDescription);
            if has_audio || has_image {
                assert!(bundle.files.contains_key("server/src/local_media.rs"));
                assert!(bundle
                    .files
                    .contains_key("web/src/lib/LocalMediaInput.svelte"));
                assert!(
                    all.contains("Optional same-host Liquid (local dev only)"),
                    "{}",
                    pack.id
                );
                assert!(all.contains("x-synthetic-workflow"), "{}", pack.id);
                assert!(all.contains("127.0.0.1"), "{}", pack.id);
                assert!(all.contains("The text-only workflow still works"));
                if has_audio {
                    audio += 1;
                    assert!(all.contains("LFM2.5-Audio-1.5B"));
                    assert!(all.contains("/api/local-media/audio"));
                    assert!(!all.contains("/api/local-media/image\", post"));
                } else {
                    image += 1;
                    assert!(all.contains("LFM2.5-VL-1.6B"));
                    assert!(all.contains("/api/local-media/image"));
                    assert!(!all.contains("/api/local-media/audio\", post"));
                }
            } else {
                ordinary += 1;
                assert!(!bundle.files.contains_key("server/src/local_media.rs"));
                assert!(!bundle
                    .files
                    .contains_key("web/src/lib/LocalMediaInput.svelte"));
                for forbidden in [
                    "LFM2.5-Audio-1.5B",
                    "LFM2.5-VL-1.6B",
                    "LIQUID_AUDIO_URL",
                    "LIQUID_VISION_URL",
                    "/api/local-media/",
                    "Optional same-host Liquid (local dev only)",
                ] {
                    assert!(!all.contains(forbidden), "{} leaked {forbidden}", pack.id);
                }
            }
        }
        assert_eq!((audio, image, ordinary), (2, 1, 14));
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
