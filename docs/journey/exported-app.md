# Exported app journey — what a clinician receives

## Method note (read this first)

This document is written **source-level, from the generator itself**, not
from a live export. It is a full read of `src/eject.rs` (2,158 lines,
current `HEAD` on this branch) plus the `post-op-monitor` pack it exercises
(`packs/post-op-monitor/pack.hcl`, `packs/post-op-monitor/scaffold/src/main.rs`,
`packs/post-op-monitor/artifact-quality.json`). No `cargo run` / live
`GET /api/apps/:id/export` was performed for this revision.

This corrects a prior version of this document. That version was written
after reading `src/eject.rs` from a *different, older worktree* — a git
worktree pinned to branch `codex/staging-preview-proof` (commit `19317c4`),
which never received the Svelte-export work. That stale file was 1,061
lines with zero Svelte references, and the prior document concluded (based
on that wrong file) that the export ships "a server-rendered Rust/axum app
plus Markdown docs... not SvelteKit or a Playwright `owned-app.mjs` test."
**That conclusion was wrong** — it was an artifact of reading the wrong
branch, not a fact about the platform. The real, current `src/eject.rs` on
this worktree (86 references to `svelte`/`Svelte`) generates a full
SvelteKit workspace and a Playwright journey test on every export, as
detailed below.

Everything below is quoted or paraphrased directly from the named
functions in `src/eject.rs` (and the `post-op-monitor` pack) so it can be
checked line-for-line against that source.

## What triggers an export

`bundle(app, pack, audit)` in `src/eject.rs:103` builds an `EjectionBundle`
— a `BTreeMap<String, String>` of relative path → file content, plus a
`python3 -c '...'` one-liner (`UNPACK_ONE_LINER`) that a clinician pipes
`export.json` through to materialize the files on disk (no zip/tar
dependency). `validate_owned_bundle` (`eject.rs:34`) enforces the shape of
every bundle before it can be imported back in as an "owned" app: a fixed
set of required paths, exactly one Markdown file (`README.md`), the three
tldraw diagrams, and a `.mcp.json` that still points at the official Svelte
MCP endpoint.

For `post-op-monitor` specifically, the pack has been converted to the
"runnable scaffold" spec (`pack.hcl`: `scaffold_path = "scaffold"`), so
`crate::packs::scaffold_sources("post-op-monitor")` returns real, compiled
Rust source (`packs/post-op-monitor/scaffold/{Cargo.toml,Cargo.lock,src/main.rs}`,
plus the synthetic seed and `artifact-quality.json`) — not a placeholder.
Those files are remapped from `scaffold/…` to `server/…` in the bundle
(`eject.rs:120-124`); the synthetic seed and quality contract keep their
pack-relative paths.

## The file tree a clinician receives

Reconstructed from every `files.insert(...)` call in `bundle()`
(`eject.rs:114-246`), for the `post-op-monitor` pack:

```
<app-id>/
├── README.md                          # generated: story + runbook + customize + design system + compliance
├── Dockerfile                         # multi-stage: builds web/, builds server/, serves both via nginx
├── pack.hcl                           # this app re-derived as the doctor's own importable template
├── .mcp.json                          # points a compatible editor at the official Svelte MCP server
├── .gitignore / .dockerignore
├── render.yaml / fly.toml             # config/deploy.yml (Kamal), nomad/job.nomad.hcl
├── config/
│   ├── nginx.conf                     # generated; single origin, proxies "/" to Rust, serves "/workspace/" statically
│   └── start.sh                       # runs the Rust binary + nginx together, forwards signals
├── diagrams/
│   ├── system-architecture.tldr
│   ├── workspace-state-machine.tldr
│   └── service-map.tldr               # all three are real tldraw v2 files, editable in tldraw
├── scripts/
│   └── reimport.mjs                   # POSTs the whole tree back to /api/apps/import as a new private starter
├── server/                            # = packs/post-op-monitor/scaffold/, real compiled Rust (issue #5 scaffold)
│   ├── Cargo.toml
│   ├── Cargo.lock
│   └── src/main.rs                    # real axum app: routes, escalation logic, sessions, audit JSONL
├── synthetic/
│   └── post-op-demo.json              # the pack's synthetic fixture (12 synthetic patients)
├── artifact-quality.json              # the pack's quality contract/rubric (job journeys, ownership, safety, a11y, docs)
└── web/                               # generated SvelteKit workspace
    ├── package.json                   # scripts: dev, build, check, "test:journey": "node tests/owned-app.mjs"
    ├── package-lock.json              # pinned; embedded via include_str!("../export-assets/web-package-lock.json")
    ├── svelte.config.js               # adapter-static, fallback 'index.html', base path "/workspace"
    ├── vite.config.ts                 # dev-server proxy for /health and /api to 127.0.0.1:8080
    ├── tsconfig.json
    ├── src/
    │   ├── app.html
    │   ├── app.d.ts
    │   ├── clinician.css              # the project-owned "hc-*" design system (independently authored, no vendored assets)
    │   ├── routes/
    │   │   ├── +layout.svelte         # imports ../clinician.css
    │   │   ├── +layout.ts             # export const ssr = false;
    │   │   └── +page.svelte           # post_op_svelte_page(): renders TreatmentWorkspace + PostOpCheckIn
    │   └── lib/
    │       ├── treatment.json         # the Gemma-selected/Rust-materialized "treatment recipe" config
    │       ├── TreatmentWorkspace.svelte
    │       └── PostOpCheckIn.svelte   # only emitted when pack.id == "post-op-monitor" (eject.rs:162-167)
    └── tests/
        └── owned-app.mjs             # generated Playwright journey test (owned_app_browser_test(), eject.rs:369)
```

Non-post-op packs that haven't been converted to the runnable-scaffold spec
get the same `web/` tree and the same generated docs, but `server/` is
absent and `Dockerfile` builds a stub that only serves `/health` — the
"Honest caveat" branch of `runbook_md()` (`eject.rs:1009-1017`) says so
explicitly in the generated README. `post-op-monitor` gets the "app source
is real" branch (`eject.rs:991-1008`).

## README.md — quoted from the generator

The final `README.md` is assembled in `bundle()` (`eject.rs:212-235`) by
concatenating, in order: `readme_md()` (the app's own story), `runbook_md()`
(get-it-running instructions, with `app/` rewritten to `server/`),
`customize_md()` (source map + how to extend it), `design_system_md()`
(the `hc-*` CSS tokens), and `compliance_md()` (gate report + attestation +
audit trail), followed by a hard-coded closing section on Gemma. Excerpts:

**Opening (`readme_md`, `eject.rs:291-341`):**

> `# {app.name}`
>
> Built on the clinician platform and ejected as an owned, self-contained
> repository. It started as one sentence:
>
> `> {app.prompt}`
>
> Scaffolded from the **{pack.name}** pack (`{pack.id}`), HIPAA controls
> pre-wired: {pack.description}

For `post-op-monitor` this renders with `pack.description` = "Recovery
tracking for surgical patients: daily pain + wound check-ins, encrypted
photo upload, escalation flags to the practice inbox." (from `pack.hcl`).

Followed by `## What the app does today (v{version})` — one bullet per
`app.features` entry — and `## Changelog — the addenda record`, which
renders every conversational edit the clinician made (`app.addenda`) as a
dated changelog entry, quoting the platform's reply and any newly wired
controls. The README's own words for this: "Every conversational edit,
logged like a chart addendum. This is the app's real history, not release
notes written after the fact."

Then `## Repository map`, naming `web/` (Svelte 5), `server/` (Rust/Axum),
`synthetic/`, `diagrams/`, `.mcp.json`, and `pack.hcl` as "this app as your
own template (`{app.id}-template`)".

**Runbook section (`runbook_md`, `eject.rs:989-1071`), real-scaffold branch
(applies to `post-op-monitor`):**

> `## The app source is real`
>
> `app/` is the Rust and Axum service. It runs the clinical workflow from
> the included `synthetic/` fixture. `web/` is the Svelte workspace for
> extending that workflow. Run the Rust service for local development:
>
> ```bash
> cd app
> APP_BIND=127.0.0.1:8080 cargo run
> cargo test
> ```
>
> In a second terminal, run the Svelte workspace:
>
> ```bash
> cd web
> npm ci
> npm run dev -- --host 127.0.0.1
> ```
>
> Open `http://127.0.0.1:5173/workspace/`. The development server sends
> `/health` and `/api/*` requests to Rust on port 8080.

(Note: in the exported README this text is emitted with `app/` and `cd app`
already rewritten to `server/` / `cd server` — `bundle()` does a literal
string replace on the whole `runbook_md`/`customize_md` output at
`eject.rs:216-227`, since the source directory really is named `server/`
in the bundle.)

It continues with unpack instructions (the `python3 -c` one-liner), a
Docker section (`docker build`, `docker run -p 8080:8080`, `curl --fail
http://127.0.0.1:8080/health`), a **"Run the browser journey"** section
(quoted verbatim below), a deploy-target table (Nomad / Render / Fly.io /
Kamal), and an "Import as a private starter" section describing
`node scripts/reimport.mjs`.

**"Run the browser journey" (`eject.rs:1036-1045`), quoted exactly:**

> `## Run the browser journey`
>
> Keep the Docker container running. In another terminal, run:
>
> ```bash
> cd web
> npm ci
> npm exec playwright install chromium
> npm run test:journey
> ```
>
> Set `OWNED_APP_URL` if the app is not at `http://127.0.0.1:8080`.
> The test saves its report and screenshots in `web/test-results/`.

**Customize section (`customize_md`, `eject.rs:1075-1163`)** gives a
"Source map" table (clinical workflow → `app/src/main.rs`; Svelte workspace
→ `web/src/routes/+page.svelte`; theme → `web/src/clinician.css`; browser
test → `web/tests/owned-app.mjs`; quality contract →
`artifact-quality.json`; pack identity → `pack.hcl`) and a 5-step "Make the
next change" checklist ending in: run `cargo fmt --check && cargo test` in
`server/`, then `npm ci && npm run check && npm run build` in `web/`, then
build/start Docker and run `npm run test:journey` against it. It closes
with a hard requirement: "A source edit is not permission to claim a
control is production-ready."

**Compliance section (`compliance_md`, `eject.rs:1249-1377`)** renders the
release status (draft vs. live + co-signer), a gate-report table (`gate |
check | HIPAA citation | basis | verdict`, one row per control such as
`phi-encryption`, `audit-log`, `ai-allowlist`, `dependency-scan`,
`auto-logoff`, `synthetic-only` for this pack), the attestation (or "None —
omitted by design" for a draft app), the append-only audit trail with
sensitive values rendered as plaintext (this is the clinician's own record,
so the tenant-side HMAC boundary from decision 0004 doesn't apply here),
and a closing "Known limitations and production responsibilities" section:
"This exported scaffold is proven against synthetic data. Any control
marked STUBBED in the gate report remains a production blocker..."

The README ends with a hard-coded closing note (`eject.rs:234`) that the
exported app "does not phone home to that planner" (Gemma) and that the
`.mcp.json` file is only an editor's connection to Svelte's public
documentation MCP server, not a second model runtime: "If you connect your
own Gemma endpoint, keep it behind the Rust API. Do not give it production
secrets, deployment authority, file access, or patient data."

## The post-op-monitor escalation flow, end to end

This is the one pack currently wired with a real scaffold, so it is the
one worth tracing precisely. Source: `packs/post-op-monitor/scaffold/src/main.rs`
and `packs/post-op-monitor/pack.hcl`.

1. **Routes** (`main.rs:963-970`): `Router::new()` wires `GET /` (home),
   `GET/POST /login`, `GET /clinician`, `POST /checkin` (server-rendered
   form path), `POST /api/checkins` (the JSON path the Svelte
   `PostOpCheckIn.svelte` component calls), `POST /photos`, `GET /health`.
2. **Demo credentials** (`main.rs:339,365-375`, deliberately labeled "not
   real secrets"): patient `demo-patient` / `learn-patient`; clinician
   `demo-clinician` / `learn-clinician`.
3. **Patient submits a check-in.** `PostOpCheckIn.svelte`
   (`eject.rs:738-903`) is a Svelte 5 component (runes: `$state`, `$props`)
   with a pain slider (0–10) plus a labeled numeric button grid, a wound
   `<select>` (`clean`/`redness`/`swelling`/`drainage`/`opening`/
   `spreading-redness`), and a note `<textarea>` (max 1,000 bytes). It
   `fetch()`s `POST /api/checkins` with an idempotency key
   (`crypto.randomUUID()`), same-origin credentials, and renders the
   server's JSON response — it never guesses the escalation outcome
   client-side ("No client-side guess... This panel changes only after the
   Rust API confirms the result").
4. **Rust decides escalation** (`main.rs:602-611`, constant at
   `main.rs:48`): `PAIN_ESCALATION_THRESHOLD = 7`. A check-in accumulates
   `reason_codes`: `"pain-threshold"` when pain ≥ 7 (message: `"pain {n}/10
   at or over threshold 7"`), and `"concerning-wound"` for wound states
   past `clean`/`redness`. If either fires, the response's
   `escalation.required` is `true`, `escalation.status` is `"queued"`,
   `escalation.destination` is `"practice-inbox"`, and an audit line
   `post_op.escalation.queued` is appended (JSONL, `main.rs:650-654`).
5. **UI reflects it**: on escalation, `PostOpCheckIn.svelte` shows "Queued
   in the synthetic practice inbox" and "Pain {n}/10 was evaluated by Rust
   and produced flag `{flag_id}`" (`eject.rs:854-857`); otherwise "Check-in
   recorded... no escalation was required."
6. **Clinician inbox.** Signing in as `demo-clinician` at `/login` and
   landing on `/clinician` (`main.rs:965-966,375`) surfaces queued flags;
   the browser test (below) asserts the text `/pain 8\/10 at or over
   threshold 7/` is visible there.
7. **Auto-logoff** (`main.rs:282,326`) — a pre-wired gate for this pack —
   ends sessions and shows: "The auto-logoff control ended this session.
   Sign in again; expired sessions never revive automatically."

`clinical_entry_path()` (`eject.rs:1439-1465`) reads the *first* `"goto"`
step of the *first* journey in `artifact-quality.json`'s
`quality.job.journeys` array to decide what the "Open the clinical
workflow" link in the Svelte page points at. For `post-op-monitor` that
first journey (`authenticated-high-pain-routes-practice-flag`) starts with
`{"do":"goto","path":"/login"}`, so the generated Svelte page's clinical
entry link and nginx's `/` → 302 redirect both point at `/login`
(`nginx_config`, `eject.rs:1467-1524`; only added when the entry isn't
already `/`).

## `web/tests/owned-app.mjs` — exact assertions

Generated by `owned_app_browser_test(pack)` (`eject.rs:369-441`), a
template string with one substitution: `__POST_OP__` becomes the literal
`true` when `pack.id == "post-op-monitor"`, else `false`
(`eject.rs:440`). Run via `npm run test:journey` → `node
tests/owned-app.mjs`. It:

- Launches headless Chromium (Playwright), one browser context, viewport
  1280×900.
- Tracks every outgoing request's hostname; anything outside
  `{127.0.0.1, localhost, <parsed hostname of $OWNED_APP_URL>}` is recorded
  as an "external host."
- Asserts `GET {baseUrl}/health` returns HTTP ok and JSON `status ===
  'ok'`.
- Navigates to `{baseUrl}/workspace/` (`waitUntil: 'networkidle'`), asserts
  the response was ok, then waits for the exact text **"Rust service
  connected"** to appear, and screenshots `web/test-results/workspace.png`
  (full page).
- **Only when `postOp` is true** (i.e., only for `post-op-monitor`), it
  drives the full escalation journey:
  1. Clicks the "Pain 8" button, clicks "Send today's check-in" (unauthenticated).
  2. Waits for and clicks the "Sign in as the synthetic patient" link.
  3. Fills `username=demo-patient`, `password=learn-patient`, clicks the
     sign-in button, waits for navigation back to `{baseUrl}/workspace/`.
  4. Clicks "Pain 8" again, clicks "Send today's check-in" again.
  5. Waits for the exact text **"Queued in the synthetic practice inbox."**
     and the pattern **`/Pain 8\/10 was evaluated by Rust/`**.
  6. Screenshots `web/test-results/patient-escalation.png` (full page).
  7. Navigates to `{baseUrl}/login`, signs in as `demo-clinician` /
     `learn-clinician`, waits for navigation to `{baseUrl}/clinician`.
  8. Waits for the pattern **`/pain 8\/10 at or over threshold 7/`** to be
     visible (case-insensitive per the regex, first match).
  9. Screenshots `web/test-results/clinician-inbox.png` (full page).
- **Zero-external-host assertion**, run regardless of `postOp`:
  `assert.deepEqual([...externalHosts], [])` with message `"The app
  contacted external hosts: {list}"` — the test fails if the exported app
  made a single request to any host other than the app itself.
- On success, writes `web/test-results/report.json` =
  `{ passed: true, baseUrl, postOp }` and logs "Owned app journey passed.
  Evidence is in {resultDir}".
- On any thrown error, screenshots `web/test-results/failure.png`, writes
  `report.json` = `{ passed: false, error }`, and rethrows (nonzero exit).
- `finally`: always closes the browser context and browser.

The corresponding entry in `package.json`
(`svelte_package_json()`, `eject.rs:343-367`) is `"test:journey": "node
tests/owned-app.mjs"`, alongside `dev`, `build` (`vite build`), and `check`
(`svelte-kit sync && svelte-check`). Dependencies are pinned:
`@sveltejs/kit` 2.69.2, `svelte` 5.56.4, `playwright` 1.61.1, `vite` 8.1.4,
etc. — and `web/package-lock.json` is the platform's own pinned lockfile
(`svelte_package_lock()` → `include_str!("../export-assets/web-package-lock.json")`),
not whatever npm would otherwise resolve, so every clinician's export
installs the exact same dependency graph the platform tested.

## The quality rubric — `artifact-quality.json`

Emitted verbatim from the pack (`packs/post-op-monitor/artifact-quality.json`,
carried through `scaffold_sources()` unchanged). It scores an export on:
`job` (40 pts — the two browser journeys above, expressed as declarative
step lists: goto/fill/select/click/assert), `ownership` (20 pts — the
required-paths list, e.g. `server/Cargo.toml`, `web/src/lib/
PostOpCheckIn.svelte`, the three `.tldr` diagrams, `nomad/job.nomad.hcl`),
`safety_honesty` (20 pts — required visible strings like "synthetic data
only" and "encryption at rest is a labeled TODO", and forbidden claims like
"HIPAA compliant" / "production ready"), `accessibility` (10 pts —
required form labels and an ARIA `main` landmark on `/`), and `docs` (10
pts — required README section headings). This is a machine-checkable
rubric the platform (and a clinician) can re-run against any future edit
of the exported app, not just the Playwright test.

## How a clinician runs it themselves

1. Download the export (`GET /api/apps/:id/export`), save as `export.json`.
2. Unpack: `mkdir -p <app-id> && cd <app-id> && <UNPACK_ONE_LINER> <
   ../export.json` (the exact one-liner is printed as `unpack` in the
   export response and documented in the README's "Unpack a downloaded
   export" section).
3. `docker build -t <app-id> . && docker run --rm -p 8080:8080 <app-id>` —
   builds Svelte (`node:22-alpine`), builds Rust (`rust:1-alpine`, `cargo
   build --release --locked`), and serves both from one
   `nginxinc/nginx-unprivileged` origin (`dockerfile()`, `eject.rs:1381-1437`;
   `nginx_config()`, `eject.rs:1467-1524`; `runtime_start_script()`,
   `eject.rs:1526-1551` runs the Rust binary and nginx as siblings under
   one PID-1-style trap/shutdown script).
4. `curl --fail http://127.0.0.1:8080/health`, then open
   `http://127.0.0.1:8080/` (redirects to the clinical entry, `/login` for
   this pack) and `http://127.0.0.1:8080/workspace/` (the Svelte extension
   workspace) — same origin, same Rust service behind both.
5. Run the browser journey exactly as the README says: `cd web && npm ci
   && npm exec playwright install chromium && npm run test:journey`,
   pointing `OWNED_APP_URL` at the running container if it isn't at
   `127.0.0.1:8080`. Evidence lands in `web/test-results/` (three
   screenshots + `report.json`).
6. Optionally deploy for real via one of the four manifests in the README
   table (Nomad / Render / Fly.io / Kamal), or re-import the whole bundle
   as a new private starter with `PRACTICE_STUDIO_URL=... node
   scripts/reimport.mjs` — which re-walks the directory tree client-side
   (skipping `.git`, build artifacts, `node_modules`, `test-results`) and
   `POST`s it to `/api/apps/import`.

## Screenshots

None are included with this revision. The Task 7b plan treats
screenshots as optional evidence reused from a live `owned-app.mjs` run
(`web/test-results/`); this revision documents from generator source only,
per the method note above, so there is no live run to capture screenshots
from. A future pass that runs `cargo run` from this worktree and drives a
real export could copy `workspace.png`, `patient-escalation.png`, and
`clinician-inbox.png` into `docs/journey/screenshots/exported-app/`.
