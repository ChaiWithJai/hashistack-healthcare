# The exported clinician deliverable

**Method used: read from generator source only.** Everything below is
transcribed or quoted directly from `src/eject.rs` (1061 lines, read in
full) and `packs/post-op-monitor/` in the main checkout
(`/Users/jaybhagat/Documents/qedc/hashistack-healthcare`, read-only). A
live export was **not** performed: the running instance on that checkout
(`target/debug/rust-proof-service`, pid 93570, listening on
`127.0.0.1:39759`, `GET /health` → `{"status":"ok",...}`) is owned by
another concurrently-active agent working on that branch. Its app store
persists to a real Postgres-backed store (`src/store.rs`), so calling
`POST /api/apps` to create a `post-op-monitor` app to export would write
a persistent record into infrastructure that agent is actively using —
not a safe or reversible side effect to trigger from here. `GET
/api/apps` on the live instance currently returns `{"apps":[]}`, so no
existing app was available to export either. This document is therefore
a precise source-level walkthrough, not an observed export; no
screenshots were captured (there was no live browser run to capture them
from).

**Important correction to the brief:** the task brief for this
investigation described the export as generating a Svelte 5/SvelteKit
frontend plus a Playwright test (`owned-app.mjs`, `npm run
test:journey`) with a zero-external-host-network assertion, referencing
commit `b3684e4` (#57). None of that exists in this codebase. `git log
--oneline` on `main` shows only three commits (`9fa12b2`, `862e4f6`,
`19317c4`); there is no `b3684e4`. `src/eject.rs` and
`packs/post-op-monitor/` contain no Svelte, no `.mjs`, no Playwright, and
no file named `owned-app.mjs`. What actually ships is a **server-rendered
Rust (axum) application** plus Markdown docs and deploy manifests, with a
JSON **quality contract** (`artifact-quality.json`) that plays the role
the brief attributed to a Playwright script. This document describes
what is actually in the repository.

## What "export" means here

`src/eject.rs`'s module doc:

> "Ejection service: one verb — bundle. Turns a doctor's app record into
> a self-contained repository they own outright: documentation generated
> from *their* record (prompt, addenda, gate report, attestation, audit
> trail), deploy manifests for four targets, and a pack.hcl derived from
> the app so what they built becomes their own re-importable template. No
> hostage code, no hostage docs (GOAL.md bars 5 and 6, issue #11)."
>
> "The bundle is a JSON file-map — zero archive dependencies; the unpack
> one-liner in the response writes it to disk with stock python3."

The API surface is `GET /api/apps/:id/export` (`src/api.rs:66`, handler
`export_app` at `src/api.rs:1488`), which returns an `EjectionBundle`:

```rust
pub struct EjectionBundle {
    pub files: BTreeMap<String, String>,   // relative path → file content
    pub unpack: String,                    // copy-paste unpack command
}
```

The unpack command embedded in the response (`unpack_command`,
`eject.rs:119`):

```
mkdir -p {app_id} && cd {app_id} && curl -s $PLATFORM/api/apps/{app_id}/export | \
python3 -c 'import json,sys,pathlib; [(lambda q: (q.parent.mkdir(parents=True,exist_ok=True), q.write_text(c))) \
(pathlib.Path(p)) for p,c in json.load(sys.stdin)["files"].items()]'
```

No archive format, no npm/git dependency to unpack it — stock `python3`
writes every file back out from the JSON map.

## The file tree a clinician receives

`bundle()` (`eject.rs:34-85`) assembles these files. For a pack that has
been converted to the "runnable scaffold" spec (`scaffold_path` set in
`pack.hcl`) — which **post-op-monitor** is — the tree is:

```
<app-id>/
├── README.md                    # the doctor's own story (prompt, features, changelog)
├── pack.hcl                     # this app reborn as a re-importable template
├── Dockerfile                   # builds app/ for real (rust:1-alpine → alpine:3)
├── render.yaml                  # Render blueprint
├── fly.toml                     # Fly.io config
├── config/deploy.yml            # Kamal deploy config
├── nomad/job.nomad.hcl          # the platform's own rendered allocation spec (or a stub, if never promoted)
├── docs/
│   ├── RUNBOOK.md               # run/deploy from this file alone
│   ├── CUSTOMIZE.md             # source map + how to make the next change
│   ├── COMPLIANCE.md            # gate report, attestation, audit trail
│   └── DESIGN_SYSTEM.md         # the opt-in clean-room clinician CSS system
├── app/                         # <- pack's scaffold/ tree, byte-identical, rooted here
│   ├── Cargo.toml                (post-op-monitor: package "post-op-monitor-scaffold")
│   ├── Cargo.lock
│   ├── src/main.rs                (the runnable axum app, ~1033 lines for post-op-monitor)
│   └── assets/clinician.css     # injected separately by eject.rs (see below)
├── synthetic/
│   └── post-op-demo.json        # the synthetic seed dataset (pack-relative path preserved)
├── artifact-quality.json        # the quality contract / "browser journey" spec (pack-relative)
├── prompts/, policies/, docs/gates/…  # any other pack-relative files (paths preserved as-is)
```

How the tree is built (`eject.rs:40-61`): `crate::packs::scaffold_sources(&pack.id)`
returns the pack's embedded source files as `(path, content)` pairs.
Anything under `scaffold/` is rehomed to `app/` (`path.strip_prefix("scaffold/")`
→ `format!("app/{rest}")`); everything else (the synthetic seed, quality
contract, prompts, policies) **keeps its pack-relative path** so the
app's own `../synthetic/…` and `include_str!` references keep resolving
identically inside and outside the platform. `eject.rs` then always adds,
regardless of pack:

- `app/assets/clinician.css` — a hand-authored "clean-room" design system
  (`clinician_design_system_css()`, `eject.rs:326-366`), explicitly **not
  wired into any page by default** ("This asset is **opt-in**... exporting
  it does not silently restyle a clinical workflow" — `eject.rs:376-378`)
- `docs/DESIGN_SYSTEM.md` — usage instructions for that CSS (two
  integration options: `include_str!` inline, or serve+link a route)
- `README.md`, `docs/RUNBOOK.md`, `docs/CUSTOMIZE.md`, `docs/COMPLIANCE.md`
- `Dockerfile`, `render.yaml`, `fly.toml`, `config/deploy.yml`,
  `nomad/job.nomad.hcl`
- `pack.hcl` (the derived, re-importable template)

There is **no `.gitignore` or `.dockerignore`** generated anywhere in
`eject.rs` — I grepped the whole file for `gitignore`/`dockerignore` and
got zero matches. That claim in the original task brief does not hold
for this codebase.

If the pack has *not* been converted to the runnable-scaffold spec (no
`scaffold_path`), the `app/` tree and `synthetic/` seed are simply
omitted; the Dockerfile instead builds a Python stub that serves
`/health` on 8080, and `docs/RUNBOOK.md` prints an "Honest caveat" instead
of "The app source is real" (`eject.rs:178-232`, tested in
`sandbox_bundle_is_draft_with_no_attestation_and_stub_job` and the
`real_source` branch of `runbook_md`). **post-op-monitor is fully
converted** (`pack.hcl`: `scaffold_path = "scaffold"`), so a clinician
exporting this pack always gets the real `app/` tree described above.

## README.md — verbatim structure (`readme_md`, `eject.rs:125-174`)

```
# {app.name}

Built on the clinician platform and ejected as an owned, self-contained
repository. It started as one sentence:

> {app.prompt}

Scaffolded from the **{pack.name}** pack (`{pack.id}`), HIPAA controls pre-wired: {pack.description}

## What the app does today (v{app.current_version})

- {each feature in app.features}

## Changelog — the addenda record

Every conversational edit, logged like a chart addendum. This is the app's
real history, not release notes written after the fact.

### v{addendum.version} — {addendum.instruction} ({utc(addendum.at)})

{addendum.reply}

- added feature: {addendum.added_feature}      (if present)
- wired controls: {addendum.wired_controls}    (if any)

## Owning it

- [docs/RUNBOOK.md](docs/RUNBOOK.md) — run and deploy this bundle, no platform access needed.
- [docs/CUSTOMIZE.md](docs/CUSTOMIZE.md) — where to make the next change and keep its quality contract green.
- [docs/COMPLIANCE.md](docs/COMPLIANCE.md) — the gate report, attestation, and audit trail.
- [pack.hcl](pack.hcl) — this app as your own template (`{app.id}-template`): re-import it,
  share it with your practice, or submit it to the registry.
```

Every field is drawn straight from the clinician's own `AppRecord` — the
prompt they typed, the features the agent scaffolded, and every
conversational addendum (edit) logged as its own dated changelog entry
with the reply the agent gave. Nothing here is generic boilerplate.

## docs/RUNBOOK.md — get it running from this file alone (`runbook_md`, `eject.rs:178-232`)

For a converted pack (post-op-monitor), the source section is:

```
## The app source is real

`app/` is this pack's runnable standalone Rust (axum) crate. It implements
the workflow described in this repository's README and boots from the
included `synthetic/` fixture. Pack-specific limitations stay visible in
the app and `docs/COMPLIANCE.md`; no generic feature is implied. Run it
directly:

cd app && cargo run    # http://127.0.0.1:8080 — or APP_BIND=host:port
cargo test             # the scaffold's own contract
```

Followed by (verbatim, with `{name}`/`{id}`/`{unpack}` substituted):

```
This bundle is self-contained. Nothing here phones home to the platform.

## Unpack (if you received the raw export JSON)

{unpack_command}

## Run with Docker

docker build -t {id} .
docker run --rm -p 8080:8080 {id}
curl http://127.0.0.1:8080/health   # → ok

## Deploy targets

| target | manifest | command |
|---|---|---|
| Nomad | `nomad/job.nomad.hcl` | `nomad job run nomad/job.nomad.hcl` |
| Render | `render.yaml` | connect the repo; Render reads the blueprint |
| Fly.io | `fly.toml` | `fly launch --copy-config --now` |
| Kamal | `config/deploy.yml` | `kamal setup` (fill in your server + registry) |

The Nomad job is the platform's own rendered allocation spec; the Vault
`{{ with secret … }}` blocks resolve against your Vault at runtime.
Render/Fly/Kamal manifests build from the Dockerfile in this bundle.

## Re-import as a template

`pack.hcl` at the bundle root is this app expressed in the platform's pack
schema — drop it into a platform's `packs/` directory (or submit it to the
registry) and "{name}" becomes a starting point instead of a one-off.
```

## docs/CUSTOMIZE.md and docs/COMPLIANCE.md

`customize_md` (`eject.rs:236-319`) gives a source map table (workflow →
`app/src/main.rs`, theme → `app/assets/clinician.css`, fixture →
`synthetic/...`, "Browser journey and quality rubric" →
`artifact-quality.json`, pack identity → `pack.hcl`), a 5-step "make the
next change" checklist ending in `cd app && cargo fmt --check && cargo
test`, and a closing line that a source edit is not permission to claim a
control is production-ready.

`compliance_md` (`eject.rs:444-572`) renders the app's release status
(`draft — not released` vs. `released — live since ..., co-signed by
...`), the gate report as a table of `gate | check | HIPAA citation |
basis | verdict`, the attestation block (co-signer, gate summary,
sha256 report digest, reviewer note — omitted entirely pre-promotion:
"None — omitted by design"), the full append-only audit trail, and a
closing "Known limitations and production responsibilities" paragraph
that names every `STUBBED` control as a production blocker. For a
*released* app the gate report is the **frozen, attestation-time**
report embedded verbatim (never a live re-run) — this is enforced by
`preflight_report()` (`eject.rs:103-117`) and covered by the test
`frozen_report_survives_even_though_the_live_app_reads_tenant_data`
(`eject.rs:1026-1060`), because a promoted app is tenant-wired and a raw
re-run against real tenant data would fail the "synthetic-only" gate
forever.

## The post-op-monitor escalation flow, end to end

Source: `packs/post-op-monitor/scaffold/src/main.rs` (1033 lines), which
becomes `app/src/main.rs` verbatim in the export.

**Roles and demo credentials** (`login_form`, line 303): the login page's
own copy reads *"Demo learning credentials (not real secrets): patient
`demo-patient / learn-patient`; clinician `demo-clinician /
learn-clinician`."*

**Patient check-in form** (`home` handler, ~line 400-478): fields are
`pain (0–10)` (number input, 0–10, default 3), `wound looks` (a `<select>`
over `KNOWN_WOUND_STATUSES = ["clean","redness","swelling","drainage",
"opening","spreading-redness"]`), and `note` (free text). There's also a
`wound photo` upload form (`multipart/form-data`) whose UI explicitly
says *"held in memory; encryption at rest is a labeled TODO (hipaa-core +
Vault transit)"*.

**Escalation logic** (`checkin` handler, line 511-599):
```rust
const PAIN_ESCALATION_THRESHOLD: u8 = 7;
const CONCERNING_WOUND_STATUSES: &[&str] = &["drainage", "opening", "spreading-redness"];
...
if form.pain >= PAIN_ESCALATION_THRESHOLD {
    reasons.push(format!("pain {}/10 at or over threshold {PAIN_ESCALATION_THRESHOLD}", form.pain));
}
if CONCERNING_WOUND_STATUSES.contains(&form.wound.as_str()) {
    reasons.push(format!("wound reported as {:?}", form.wound));
}
let flagged = !reasons.is_empty();
if flagged {
    state.0.inbox.lock().unwrap().push(Flag { patient_id, reason: reasons.join("; "), at: unix_now() });
}
```
Pain ≥ 7/10, **or** a concerning wound status, pushes a `Flag` into an
in-process `inbox: Mutex<Vec<Flag>>`. The check-in confirmation page then
says *"flag routed to the practice inbox: {reasons}"*, or *"within
expected recovery range — no escalation"* if not flagged.

**Clinician inbox** (`clinician` handler, line 481-500): role-gated
(`auth.role != Role::Clinician` → 403 "clinician role required"),
renders every flag in the inbox, newest first: *"{patient_id} — {reason}
at {timestamp}"*, with the page copy *"Signed in as {actor}. Patients
cannot access this route."*

**Every check-in writes an audit JSONL line** (comment at line 562-563:
*"a flag over threshold must route to the practice inbox — not a
dashboard nobody watches"*), streamed to stdout in production
(`AuditSink::Stdout`) or captured in memory for the scaffold's own test
suite. This is covered by the scaffold's own `#[cfg(test)]` module — e.g.
`checkin_over_threshold_flags_inbox_and_writes_an_audit_jsonl_line`
(around line 847) asserts *"pain 8 + drainage routes one flag"* and that
the reason string contains both `"pain 8/10"` and `"drainage"`; a sibling
test `in_range_checkin_does_not_flag` asserts the inbox stays empty for
in-range values.

The dataset is refused at boot if it isn't marked synthetic
(`AppState::from_dataset`, line 186-197): *"refusing to boot: dataset
{...} is not marked SYNTHETIC — this scaffold only ever sees synthetic
data."*

## The "browser journey" quality contract (not a Playwright test)

There is no `owned-app.mjs`. What plays that role is
`artifact-quality.json`, which is one of the pack-relative files that
rides into the export bundle unmodified (confirmed by the repository's
own test, `every_built_in_pack_bundle_carries_real_source_and_quality_contract`,
which asserts `bundle.files.contains_key("artifact-quality.json")`).
Verbatim content for post-op-monitor:

```json
{
  "schema_version": 1,
  "pack": "post-op-monitor",
  "runtime": {"manifest":"app/Cargo.toml","binary":"app","health_path":"/health","synthetic":"synthetic/post-op-demo.json"},
  "quality": {
    "job": {"weight":40,"journeys":[{"id":"authenticated-high-pain-routes-practice-flag","steps":[
      {"do":"goto","path":"/login"},
      {"assert":"visible","text":"Demo learning credentials"},
      {"do":"fill","label":"username","value":"demo-patient"},
      {"do":"fill","label":"password","value":"learn-patient"},
      {"do":"click","role":"button","name":"sign in"},
      {"do":"fill","label":"pain (0–10)","value":"9"},
      {"do":"select","label":"wound looks","value":"clean"},
      {"do":"fill","label":"note","value":"much worse since last night"},
      {"do":"click","role":"button","name":"check-in"},
      {"assert":"visible","text":"flag routed to the practice inbox"},
      {"do":"goto","path":"/"},
      {"assert":"visible","text":"pain 9/10"},
      {"assert":"log_count","path":"/checkin","equals":1}
    ]}]},
    "ownership": {"weight":20,"required_paths":["app/Cargo.toml","app/src/main.rs","pack.hcl","artifact-quality.json","README.md","docs/RUNBOOK.md","docs/COMPLIANCE.md","nomad/job.nomad.hcl"]},
    "safety_honesty": {"weight":20,"required_visible":["synthetic data only","encryption at rest is a labeled TODO","hipaa-core placeholder"],"forbidden_claims":["HIPAA compliant","production ready"]},
    "accessibility": {"weight":10,"path":"/","required_labels":["pain (0–10)","wound looks","note"],"required_landmarks":["main"]},
    "docs": {"weight":10,"required_sections":{"README.md":["What the app does today","Owning it"],"docs/RUNBOOK.md":["Run with Docker","Deploy targets"],"docs/COMPLIANCE.md":["Gate report","Known limitations"]}}
  }
}
```

This is a declarative journey/rubric spec, scored across five weighted
dimensions (job / ownership / safety-honesty / accessibility / docs) —
not a browser-automation script a clinician runs directly, and there is
**no zero-external-network-request assertion anywhere in this repository**
(I grepped for `externalHosts`, `playwright`, `Playwright`, and
`test:journey` across the whole main checkout and found no matches in
either `src/` or `packs/`). Whatever consumes `artifact-quality.json` to
actually drive a browser and score these journeys lives outside
`src/eject.rs`/`packs/post-op-monitor/` — it was not present in the parts
of the codebase I read, and I did not find it elsewhere in a full-repo
grep. The closest thing that exists is `evals/` (`evals/harness/run.mjs`,
`evals/journey/profile.mjs`), but that is the platform's own *scaffolding
evaluation harness* used when packs are being authored/scored — it is not
part of the ejected bundle and is not referenced by `eject.rs`.

## How a clinician runs and verifies their export themselves

Exact commands, per `docs/RUNBOOK.md` as generated:

```bash
# 1. Unpack (if you received the raw JSON export instead of a git checkout)
mkdir -p <app-id> && cd <app-id> && \
  curl -s $PLATFORM/api/apps/<app-id>/export | \
  python3 -c 'import json,sys,pathlib; [(lambda q: (q.parent.mkdir(parents=True,exist_ok=True), q.write_text(c))) \
  (pathlib.Path(p)) for p,c in json.load(sys.stdin)["files"].items()]'

# 2. Run the real app source directly
cd app && cargo run       # http://127.0.0.1:8080 (or set APP_BIND=host:port)
cargo test                # the scaffold's own contract — includes the
                           # escalation-flag tests quoted above

# 3. Or run it in Docker (works even without a Rust toolchain)
docker build -t <app-id> .
docker run --rm -p 8080:8080 <app-id>
curl http://127.0.0.1:8080/health   # → ok

# 4. Walk the escalation journey by hand, matching artifact-quality.json's
#    "authenticated-high-pain-routes-practice-flag" journey:
#      - open /login, sign in as demo-patient / learn-patient
#      - submit a check-in with pain=9, wound=clean, any note
#      - confirm the page shows "flag routed to the practice inbox"
#      - log out, sign in as demo-clinician / learn-clinician
#      - open /clinician and confirm the flag appears in the practice inbox

# 5. Deploy for real, pick one manifest already in the bundle:
nomad job run nomad/job.nomad.hcl        # or
# connect the repo to Render (reads render.yaml), or
fly launch --copy-config --now           # or
kamal setup                              # after filling in server + registry in config/deploy.yml
```

`docs/COMPLIANCE.md` and `docs/CUSTOMIZE.md` are the two documents to
read before making the app someone's real production tool: COMPLIANCE
lists every `STUBBED` gate as a production blocker (verbatim: *"Any
control marked STUBBED in the gate report remains a production blocker:
configure authenticated user access, encryption at rest, durable audit
retention, approved runtime egress, backups, and incident response
before handling real patient information"*), and CUSTOMIZE's closing
paragraph repeats the same point in the context of extending the app:
*"Before real patient use, replace process-local state and demo
credentials, configure durable audit/storage/backups, enforce workload
identity and egress, and repeat the gate review under the intended BAA
boundary."*

## Screenshots

None. No live export or browser run was performed (see "Method used"
above), so there is nothing in `web/test-results/` or elsewhere to
capture. `docs/journey/screenshots/exported-app/` was intentionally left
empty/uncreated for this task.
