# Treatment report: SvelteKit

## Design in five sentences

Everything reactive is a `$state` rune or a `writable` store, so there is
no separate "re-render" mental model to hold — assigning to a variable IS
the update. Routing is purely file-based (`src/routes/**/+page.svelte`),
so the four screens map 1:1 onto four files with no router config to
write or a route table to keep in sync. There is no virtual DOM: the
Svelte compiler turns each `.svelte` file into surgical DOM-update
instructions at build time, which is why the dev server and hot-reload
feel instantaneous even with a 2-second polling interval running.
Server-side rendering is opt-out, not opt-in, which is a real design
decision, not a footnote — see "what fought the framework" below.
Co-location is the default posture: markup, script, and scoped `<style>`
live in one file per screen, and a store file is only needed the moment
state must survive a component teardown (the polling interval).

## Footprint

- **Files (excluding `node_modules`/`.svelte-kit`):** 21 files, 2,445
  lines total (includes scaffold boilerplate: `.gitignore`, `.npmrc`,
  `README.md`, `package-lock.json`, `tsconfig.json`).
- **Hand-written application code:** `src/lib/api.ts` (117 lines),
  `src/lib/appStore.ts` (13 lines), `src/lib/stages.ts` (19 lines), 4
  route files (`+page.svelte` × 4, builder/workflow-rail/gate/audit),
  `src/app.css` (importing Task 1's `tokens.css`).
- **`package.json` dependency count:** 8 (all `devDependencies`:
  `@sveltejs/adapter-auto`, `@sveltejs/kit`, `@sveltejs/vite-plugin-svelte`,
  `svelte`, `svelte-check`, `typescript`, `vite`, `vite-plugin-singlefile`).
  No runtime dependencies — SvelteKit ships nothing to the client beyond
  the compiled output.
- **`npm install` tree:** 57 top-level packages under `node_modules`.
- **Before running `npm run dev`:** copy `.env.example` to `.env` (sets
  `VITE_DEV_TOKEN`, the dev-only bearer token) — `src/lib/api.ts` throws
  at startup if it's unset.
- **Dev-server startup time observed in Step 6:** `VITE v8.1.4 ready in
  406 ms` (cold) / `607 ms` (warm restart after a code fix). Both under a
  second — Vite's dependency pre-bundling plus Svelte's small compiler
  output keep this fast even with SvelteKit's file-based routing layered
  on top.

## What worked / what fought the framework

**Worked:**
- File-based routing meant the four required screens (`+page.svelte`,
  `apps/[id]/+page.svelte`, `apps/[id]/gate/+page.svelte`,
  `apps/[id]/audit/+page.svelte`) were just... the four files the brief
  asked for, with zero router wiring.
- `$state` + reactive `{#if}`/`{#each}` blocks made the gate report and
  audit trail screens (list of items, conditional loading state) close to
  boilerplate-free — no `useEffect`-equivalent dependency arrays to get
  wrong.
- TypeScript inference through `api.ts`'s typed `request<T>()` wrapper
  flowed cleanly into every `.svelte` file's `$state<App | null>()` etc.,
  with no manual annotation needed at the call sites.
- Hot-reload survived edits to the gate and audit pages while the
  workflow-rail page's 2-second poll was actively running elsewhere in
  the app — Vite's HMU boundary is per-component, so the running interval
  in a sibling tab wasn't disturbed.

**Fought the framework — one real bug, not just friction:**
- SvelteKit prerenders/SSRs every route on first load by default. The
  workflow-rail page originally called `pollApp(id)` (which does an
  immediate `fetch`) directly in the component's top-level `<script>`,
  matching the brief's Step 4/5 snippet literally. That fetch used a
  relative URL (`/api/apps/:id`), and SvelteKit's SSR runtime throws hard
  on that ("Cannot call `fetch` eagerly during server-side rendering with
  a relative URL") — the *first* request the crash reached was over HTTP,
  but the *uncaught exception it produced actually killed the whole Node
  dev-server process*, taking every other route down with it until
  restarted. Fix: gate the store creation on `browser` from
  `$app/environment`, falling back to an inert store during SSR (see
  `src/routes/apps/[id]/+page.svelte`). This is the one place where "a
  separate store file for polling" (as the brief anticipated) wasn't the
  friction — the friction was that co-locating an eager side effect in
  page-script scope is unsafe by default in SvelteKit, and nothing in the
  compiler or dev-server warns you until the request actually lands.
  Every other screen already used `onMount`, so the bug was isolated to
  the one file the brief's own snippet nudged toward the unsafe pattern.

**API-contract mismatch (not a SvelteKit issue, but blocked initial
verification):** the brief's illustrative `api.ts`/`GateReport`/
`AuditEntry` shapes (`pack_id`, `state` as one of 8 named stages,
`checks[].passed`, flat audit array) don't match the real, running
`src/api.rs`/`src/state.rs`. The real API uses `pack` + `stage:
"sandbox"|"live"`, `create_app` takes `{ prompt, pack, name? }` and
returns `{ app, scaffold }`, gate reports nest under `{ report, meter,
reviewer_note }` with `results[].status: "pass"|"stubbed"|"fail"`, and
audit is `{ events: [...] }` with `seq`/`actor`/`action`/`detail` fields.
`src/lib/api.ts` and `src/lib/stages.ts` document this divergence inline
and implement the real contract (`stages.ts` maps the real 2-value
lifecycle onto the brief's 8 display labels as a display heuristic, not
a field the API tracks) — this was necessary for Step 6's live
verification to be possible at all, and Tasks 3–4 will hit the identical
mismatch if they implement the brief's snippet literally.

## Step 6 verification notes

`cargo run` (bound to `127.0.0.1:3000` per `APP_BIND` default in
`src/main.rs`) hung indefinitely in this session — zero CPU after 6+
minutes, no `rustc` child process, empty stdout/stderr log, and a
`sample`-tool stack trace showing it blocked in
`fingerprint::__compare_old_fingerprint` doing a plain file `read()` on
the target directory's fingerprint cache. This is an environment-local
hang (likely sandboxed filesystem I/O stalling on that specific read),
not a code issue — the same commit (`19317c4`) was already running
correctly as two other long-lived local instances (verified via
`/health`, one from this repo's own `target/debug/rust-proof-service`
started ~1 hour earlier). Verification was performed against that
already-running, same-commit instance (proxied at its actual bind port)
rather than waiting on the hung process; the committed `vite.config.ts`
still points at the spec's `127.0.0.1:3000` for the shipped treatment.
Verified via `curl` through the Vite proxy and via a real browser
(Playwright): pack list and existing apps render on the Builder; app
creation round-trips through the real `{ prompt, pack, name }` contract;
the workflow rail polls and renders `stage`/`pack`/`current_version` with
the 8-label heuristic; the gate report renders 5 pass / 1 fail
(`auto-logoff after idle`) with citations and a working "Fix" affordance;
the audit trail renders the real reverse-chronological event stream
(`app.created`, `agent.routed`, `agent.attempt`, `agent.scaffolded`) with
actor/detail. No console errors during the browser pass.

## What I'd steal from the other treatments

(filled after Tasks 3–4)
