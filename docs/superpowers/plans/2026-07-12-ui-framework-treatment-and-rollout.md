# UI Framework Treatment + Rollout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the same 4 Pareto-critical doctor-workflow screens in SvelteKit, Nuxt, and SolidJS+TanStack Start against the real Rust API, decide a winner on ecosystem/primitives/ease-of-shipping, then rebuild the *whole* 8-stage workflow in the winner and make it the served UI (replacing `web/index.html`), with before/after screenshots and GitHub issue/PR artifacts as evidence.

**Architecture:** Three throwaway Vite-based SPAs (`web-treatments/{svelte,nuxt,solid-tanstack}/`) each call the existing `axum` API (`cargo run`, `127.0.0.1:3000`, bearer token `dev-token-osei`) directly — no mock data, no backend changes for the treatment phase. Each treatment ships an API client + a polling/reactive store + 4 screens + a self-report. A comparison doc scores and picks a winner. The winner is then rebuilt for full 8-stage parity and built as a single self-contained `index.html` (via `vite-plugin-singlefile`, since `src/api.rs` embeds it at compile time with `include_str!("../web/index.html")`), replacing the current file. Screenshots of old vs. new are captured with Chrome automation. A draft PR is opened and closed (evidence, not merged raw); a parent GitHub issue + sub-issues reference it.

**Tech Stack:** SvelteKit (Task 2), Nuxt 3/Vue (Task 3), SolidJS + `@tanstack/solid-router`/`@tanstack/solid-query` (Task 4). All three: Vite, TypeScript, `vite-plugin-singlefile` for the final build. Backend: existing Rust/axum, unchanged except the doctor route in the rollout task.

## Global Constraints

- No React, no Next.js (user exclusion).
- shakti-ui component *code* is not vendored (ADR 0007); only its design tokens are, per the written clearance recorded in ADR 0008 (Task 1).
- Every screen hits the real `cargo run` API — no mocked fixtures.
- Auth: `Authorization: Bearer dev-token-osei` header (staging/identities.hcl maps this to `dr-osei`, a Clinician). Never hardcode this token outside `.env`/gitignored local config; document it in each treatment's README as a dev-only value.
- Branch: `claude/treatment-planning-ui-frameworks`, based on `origin/main`. Do not touch `codex/*` branches. Rebase (not merge) onto `main` if `main` moves before Task 6 lands.
- The final build output at `web/index.html` must remain a single file (the Rust binary embeds it via `include_str!`).

---

### Task 1: ADR 0008 + shared design tokens

**Files:**
- Create: `docs/decisions/0008-shakti-ui-vendoring-clearance.md`
- Create: `web-treatments/tokens/tokens.css`
- Create: `web-treatments/tokens/README.md`

**Interfaces:**
- Produces: `web-treatments/tokens/tokens.css` — CSS custom properties (`--st-*` prefix) every treatment `@import`s. Exact token names defined in this task; Tasks 2–4 consume them unchanged.

- [ ] **Step 1: Extract the archive's design tokens**

Read `/Users/jaybhagat/Downloads/Lovable for clinicians on HashiStack.zip`'s
`_ds/shakti-ui-*/_ds_manifest.json` and `_ds_bundle.css` (unzip to a scratch
dir first, e.g. `/private/tmp/claude-501/.../scratchpad/shakti-extract/`).
Identify the semantic token set: colors (brand/ink/muted/surface/success/
warn/danger + their `-dark`/`-bg` pairs), spacing scale, radii, font
stack, focus-ring treatment. Do not copy component markup/JS — tokens
(CSS custom properties and raw values) only.

- [ ] **Step 2: Write `web-treatments/tokens/tokens.css`**

```css
/* Shared design tokens extracted from shakti-ui per ADR 0008 written
   clearance. Values only — no vendored component code. */
:root {
  --st-ink: #2c2528;
  --st-muted: #75696d;
  --st-line: #e7dadd;
  --st-panel: #fffdfc;
  --st-canvas: #fbf6f3;
  --st-brand: #9f3d5f;
  --st-brand-dark: #762844;
  --st-focus: #d76b8e;
  --st-success: #287052;
  --st-success-bg: #edf7f1;
  --st-warn: #8b571e;
  --st-warn-bg: #fff5df;
  --st-danger: #a43c3c;
  --st-danger-bg: #fff0ee;
  --st-radius-control: 12px;
  --st-radius-card: 18px;
  --st-radius-dialog: 24px;
  --st-space-1: 4px;
  --st-space-2: 8px;
  --st-space-3: 12px;
  --st-space-4: 20px;
  --st-font: ui-rounded, "Nunito Sans", ui-sans-serif, system-ui,
    -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
/* Replace the placeholder hex/scale values above with the real
   extracted values from Step 1 before committing — this is a starting
   point, not the final token set. */
```

Adjust the actual values to what Step 1 found (the values above mirror
`web/index.html`'s existing independently-authored theme as a fallback if
the archive's tokens turn out to overlap closely — prefer the archive's
real extracted values where they differ).

- [ ] **Step 3: Write `web-treatments/tokens/README.md`**

Document: where these values came from (the zip's SHA-256 already on
record in ADR 0007), that this file is hand-extracted values (not the
compiled `_ds_bundle.css`/`.js`, which stay out of the repo), and that
all three treatments `@import "../tokens/tokens.css"` and use only
`var(--st-*)` — never inline the archive's own class names.

- [ ] **Step 4: Write ADR 0008**

```markdown
# ADR 0008: Shakti-ui design-token vendoring — written clearance obtained

Status: accepted

ADR 0007 barred vendoring the supplied Shakti/Catalyst archive because its
compiled kit carries Tailwind Plus redistribution restrictions. Written
clearance to vendor has since been obtained (2026-07-12, on file with the
project owner) — this clearance covers extracting and vendoring the
archive's **design tokens** (`web-treatments/tokens/tokens.css`): color,
spacing, radius, type-stack, and focus-ring values.

It does not cover the archive's compiled JavaScript, component markup,
Catalyst JSX source, fonts, or demo assets, none of which are vendored —
component *implementations* in `web-treatments/*` and the eventual `web/`
rollout remain independently authored per framework, consuming only the
token layer. ADR 0007's provenance record (archive SHA-256) still applies.
```

- [ ] **Step 5: Commit**

```bash
git add docs/decisions/0008-shakti-ui-vendoring-clearance.md web-treatments/tokens/
git commit -m "docs: ADR 0008 — shakti-ui token vendoring clearance + shared tokens"
```

---

### Task 2: Treatment — SvelteKit

**Files:**
- Create: `web-treatments/svelte/` (SvelteKit + TypeScript + Vite project)
- Create: `web-treatments/svelte/src/lib/api.ts`
- Create: `web-treatments/svelte/src/lib/appStore.ts`
- Create: `web-treatments/svelte/src/routes/+page.svelte` (Builder)
- Create: `web-treatments/svelte/src/routes/apps/[id]/+page.svelte` (Workflow rail)
- Create: `web-treatments/svelte/src/routes/apps/[id]/gate/+page.svelte` (Gate report)
- Create: `web-treatments/svelte/src/routes/apps/[id]/audit/+page.svelte` (Audit trail)
- Create: `docs/treatments/ui-svelte.md`

**Interfaces:**
- Consumes: `web-treatments/tokens/tokens.css` (Task 1).
- Produces: pattern other treatments are compared against in Task 5 —
  same 4 screens, same API surface, same auth header approach.

- [ ] **Step 1: Scaffold**

```bash
cd web-treatments
npx sv create svelte --template minimal --types ts --no-install
cd svelte && npm install
npm install -D vite-plugin-singlefile
```

- [ ] **Step 2: Configure the dev proxy and import shared tokens**

`web-treatments/svelte/vite.config.ts`:
```ts
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    proxy: { '/api': 'http://127.0.0.1:3000', '/health': 'http://127.0.0.1:3000' }
  }
});
```

`web-treatments/svelte/src/app.css`:
```css
@import "../../tokens/tokens.css";
body { background: var(--st-canvas); color: var(--st-ink); font-family: var(--st-font); }
```

- [ ] **Step 3: Write the API client**

`web-treatments/svelte/src/lib/api.ts`:
```ts
const TOKEN = import.meta.env.VITE_DEV_TOKEN ?? 'dev-token-osei';

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api${path}`, {
    ...init,
    headers: { Authorization: `Bearer ${TOKEN}`, 'Content-Type': 'application/json', ...(init?.headers ?? {}) }
  });
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
  return res.status === 204 ? (undefined as T) : res.json();
}

export interface Pack { id: string; name: string; }
export interface App { id: string; name: string; pack_id: string; state: string; }
export interface GateReport { app_id: string; checks: { id: string; name: string; passed: boolean }[]; }
export interface AuditEntry { id: string; kind: string; at: string; detail: string; }

export const api = {
  listPacks: () => request<Pack[]>('/packs'),
  listApps: () => request<App[]>('/apps'),
  createApp: (body: { name: string; pack_id: string; description: string }) =>
    request<App>('/apps', { method: 'POST', body: JSON.stringify(body) }),
  getApp: (id: string) => request<App>(`/apps/${id}`),
  iterate: (id: string, instruction: string) =>
    request<App>(`/apps/${id}/iterate`, { method: 'POST', body: JSON.stringify({ instruction }) }),
  gateReport: (id: string) => request<GateReport>(`/apps/${id}/gate`),
  fixGate: (id: string, gateId: string) =>
    request<GateReport>(`/apps/${id}/gate/${gateId}/fix`, { method: 'POST' }),
  promote: (id: string) => request<App>(`/apps/${id}/promote`, { method: 'POST' }),
  rollback: (id: string) => request<App>(`/apps/${id}/rollback`, { method: 'POST' }),
  audit: (id: string) => request<AuditEntry[]>(`/apps/${id}/audit`)
};
```

- [ ] **Step 4: Write the polling store**

`web-treatments/svelte/src/lib/appStore.ts`:
```ts
import { writable } from 'svelte/store';
import { api, type App } from './api';

export function pollApp(id: string, intervalMs = 2000) {
  const store = writable<App | null>(null);
  let timer: ReturnType<typeof setInterval>;
  async function tick() { store.set(await api.getApp(id)); }
  tick();
  timer = setInterval(tick, intervalMs);
  return { subscribe: store.subscribe, stop: () => clearInterval(timer) };
}
```

- [ ] **Step 5: Build the 4 screens**

`+page.svelte` (Builder): a form (`name`, `pack_id` select populated from
`api.listPacks()`, `description` textarea) that calls `api.createApp` on
submit and lists existing apps from `api.listApps()` below it, each
linking to `/apps/[id]`.

`apps/[id]/+page.svelte` (Workflow rail): uses `pollApp` from Step 4,
renders the 8 stage names (`describe, generate, preview, iterate, gate,
deploy, operate, audit`) with the current one highlighted from
`app.state`, an iterate textarea + submit calling `api.iterate`, and
promote/rollback buttons calling `api.promote`/`api.rollback`.

`apps/[id]/gate/+page.svelte` (Gate report): loads `api.gateReport(id)`,
renders each check with pass/fail styling (`var(--st-success)` /
`var(--st-danger)`), a "Fix" button per failing check calling
`api.fixGate(id, check.id)` that re-fetches the report on success.

`apps/[id]/audit/+page.svelte` (Audit trail): loads `api.audit(id)`,
renders a reverse-chronological list, each entry showing `kind`, `at`,
`detail`.

- [ ] **Step 6: Verify against the live API**

```bash
cargo run &   # from repo root, in a separate terminal/session
cd web-treatments/svelte && npm run dev
```
Open the dev server URL, create an app via the Builder, confirm the
workflow rail polls and updates, confirm gate/audit screens render real
data. Kill `cargo run` when done.

- [ ] **Step 7: Write the self-report**

`docs/treatments/ui-svelte.md`, following the format in
`docs/process/gitops-treatments.md`:
- **Design in five sentences** — stores-as-reactivity, file-based routing,
  no virtual DOM.
- **Footprint** — `git diff --stat` vs the branch point, `package.json`
  dependency count, dev-server startup time observed in Step 6.
- **What worked / what fought the framework** — concrete, e.g. SvelteKit's
  `+page.svelte` co-location vs. needing a separate store file for
  polling; TypeScript inference quality; hot-reload behavior while a
  poll interval is running.
- **What I'd steal from the other treatments** — leave as `(filled after
  Tasks 3–4)`.

- [ ] **Step 8: Commit**

```bash
git add web-treatments/svelte docs/treatments/ui-svelte.md
git commit -m "treatment: SvelteKit Pareto screens against live API"
```

---

### Task 3: Treatment — Nuxt (Vue)

**Files:**
- Create: `web-treatments/nuxt/` (Nuxt 3 + TypeScript project)
- Create: `web-treatments/nuxt/composables/useApi.ts`
- Create: `web-treatments/nuxt/composables/usePollApp.ts`
- Create: `web-treatments/nuxt/pages/index.vue` (Builder)
- Create: `web-treatments/nuxt/pages/apps/[id]/index.vue` (Workflow rail)
- Create: `web-treatments/nuxt/pages/apps/[id]/gate.vue` (Gate report)
- Create: `web-treatments/nuxt/pages/apps/[id]/audit.vue` (Audit trail)
- Create: `docs/treatments/ui-nuxt.md`

**Interfaces:**
- Consumes: `web-treatments/tokens/tokens.css` (Task 1). Same `App`,
  `Pack`, `GateReport`, `AuditEntry` shapes as Task 2's `api.ts` (copy the
  interfaces verbatim so Task 5's comparison is apples-to-apples).
- Produces: second data point for Task 5.

- [ ] **Step 1: Scaffold**

```bash
cd web-treatments
npx nuxi@latest init nuxt --package-manager npm --gitInit=false
cd nuxt && npm install
```

- [ ] **Step 2: Configure the dev proxy and import shared tokens**

`web-treatments/nuxt/nuxt.config.ts`:
```ts
export default defineNuxtConfig({
  devtools: { enabled: false },
  css: ['~/assets/app.css'],
  nitro: {
    devProxy: {
      '/api': { target: 'http://127.0.0.1:3000/api', changeOrigin: true }
    }
  },
  runtimeConfig: {
    public: { devToken: process.env.NUXT_PUBLIC_DEV_TOKEN ?? 'dev-token-osei' }
  }
});
```

`web-treatments/nuxt/assets/app.css`:
```css
@import "../../tokens/tokens.css";
body { background: var(--st-canvas); color: var(--st-ink); font-family: var(--st-font); }
```

- [ ] **Step 3: Write the API composable**

`web-treatments/nuxt/composables/useApi.ts`:
```ts
export interface Pack { id: string; name: string }
export interface App { id: string; name: string; pack_id: string; state: string }
export interface GateReport { app_id: string; checks: { id: string; name: string; passed: boolean }[] }
export interface AuditEntry { id: string; kind: string; at: string; detail: string }

export function useApi() {
  const { public: { devToken } } = useRuntimeConfig();
  async function request<T>(path: string, init?: RequestInit): Promise<T> {
    const res = await fetch(`/api${path}`, {
      ...init,
      headers: { Authorization: `Bearer ${devToken}`, 'Content-Type': 'application/json', ...(init?.headers ?? {}) }
    });
    if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
    return res.status === 204 ? (undefined as T) : res.json();
  }
  return {
    listPacks: () => request<Pack[]>('/packs'),
    listApps: () => request<App[]>('/apps'),
    createApp: (body: { name: string; pack_id: string; description: string }) =>
      request<App>('/apps', { method: 'POST', body: JSON.stringify(body) }),
    getApp: (id: string) => request<App>(`/apps/${id}`),
    iterate: (id: string, instruction: string) =>
      request<App>(`/apps/${id}/iterate`, { method: 'POST', body: JSON.stringify({ instruction }) }),
    gateReport: (id: string) => request<GateReport>(`/apps/${id}/gate`),
    fixGate: (id: string, gateId: string) =>
      request<GateReport>(`/apps/${id}/gate/${gateId}/fix`, { method: 'POST' }),
    promote: (id: string) => request<App>(`/apps/${id}/promote`, { method: 'POST' }),
    rollback: (id: string) => request<App>(`/apps/${id}/rollback`, { method: 'POST' }),
    audit: (id: string) => request<AuditEntry[]>(`/apps/${id}/audit`)
  };
}
```

- [ ] **Step 4: Write the polling composable**

`web-treatments/nuxt/composables/usePollApp.ts`:
```ts
export function usePollApp(id: string, intervalMs = 2000) {
  const api = useApi();
  const app = ref<Awaited<ReturnType<typeof api.getApp>> | null>(null);
  let timer: ReturnType<typeof setInterval>;
  async function tick() { app.value = await api.getApp(id); }
  onMounted(() => { tick(); timer = setInterval(tick, intervalMs); });
  onUnmounted(() => clearInterval(timer));
  return app;
}
```

- [ ] **Step 5: Build the 4 screens**

`pages/index.vue` (Builder): same fields as Task 2 Step 5's Builder,
using `useApi().createApp` / `listApps` / `listPacks`, Vue `ref`s +
`<form @submit.prevent>`.

`pages/apps/[id]/index.vue` (Workflow rail): `usePollApp` from Step 4,
same 8-stage rail rendering, iterate form, promote/rollback buttons.

`pages/apps/[id]/gate.vue` (Gate report): `useApi().gateReport`, same
pass/fail rendering and per-check fix button.

`pages/apps/[id]/audit.vue` (Audit trail): `useApi().audit`, same
reverse-chronological list.

- [ ] **Step 6: Verify against the live API**

```bash
cargo run &
cd web-treatments/nuxt && npm run dev
```
Same manual verification as Task 2 Step 6. Kill `cargo run` when done.

- [ ] **Step 7: Write the self-report**

`docs/treatments/ui-nuxt.md`, same format as Task 2 Step 7, but Vue-
specific observations: `ref`/`computed` ergonomics vs. Svelte stores,
Nuxt's file-based routing + auto-imports (no manual `import ref from
'vue'`), SSR-by-default implications for a client-polling screen
(did you need `<ClientOnly>` or `import.meta.client` guards?).

- [ ] **Step 8: Commit**

```bash
git add web-treatments/nuxt docs/treatments/ui-nuxt.md
git commit -m "treatment: Nuxt Pareto screens against live API"
```

---

### Task 4: Treatment — SolidJS + TanStack Start/Router

**Files:**
- Create: `web-treatments/solid-tanstack/` (Vite + Solid + TanStack Router project)
- Create: `web-treatments/solid-tanstack/src/lib/api.ts`
- Create: `web-treatments/solid-tanstack/src/lib/pollApp.ts`
- Create: `web-treatments/solid-tanstack/src/routes/index.tsx` (Builder)
- Create: `web-treatments/solid-tanstack/src/routes/apps.$id.tsx` (Workflow rail)
- Create: `web-treatments/solid-tanstack/src/routes/apps.$id.gate.tsx` (Gate report)
- Create: `web-treatments/solid-tanstack/src/routes/apps.$id.audit.tsx` (Audit trail)
- Create: `docs/treatments/ui-solid-tanstack.md`

**Interfaces:**
- Consumes: `web-treatments/tokens/tokens.css` (Task 1). Same API shapes
  as Task 2/3.
- Produces: third data point for Task 5, plus an explicit maturity-risk
  note (Solid support in TanStack Start is experimental) that Task 5's
  scoring must weigh.

- [ ] **Step 1: Scaffold and record the maturity check**

```bash
cd web-treatments
npm create vite@latest solid-tanstack -- --template solid-ts
cd solid-tanstack && npm install
npm install @tanstack/solid-router @tanstack/solid-query
npm install -D vite-plugin-singlefile @tanstack/router-plugin
```

Check `@tanstack/solid-router`'s current README/CHANGELOG for its
stability disclaimer (record the exact wording verbatim in Step 6's
self-report — this is evidence, not paraphrase). If `@tanstack/
solid-router` does not exist or is unusable (published API broken,
no Solid adapter shipped), fall back within this same task to
`@tanstack/solid-query` + Solid's own built-in router (`@solidjs/
router`) for navigation, and record that substitution as the first
finding in the self-report — do not silently swap frameworks.

- [ ] **Step 2: Configure the dev proxy and import shared tokens**

`web-treatments/solid-tanstack/vite.config.ts`:
```ts
import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solid()],
  server: { proxy: { '/api': 'http://127.0.0.1:3000' } }
});
```

`web-treatments/solid-tanstack/src/app.css`:
```css
@import "../../tokens/tokens.css";
body { background: var(--st-canvas); color: var(--st-ink); font-family: var(--st-font); }
```

- [ ] **Step 3: Write the API client**

`web-treatments/solid-tanstack/src/lib/api.ts`: identical contract to
Task 2 Step 3's `api.ts` (same interfaces and function names), adapted
only for import syntax — copy it, don't redesign it, so the comparison
in Task 5 isolates framework differences rather than API-shape
differences.

- [ ] **Step 4: Write the polling primitive**

`web-treatments/solid-tanstack/src/lib/pollApp.ts`:
```ts
import { createSignal, onCleanup } from 'solid-js';
import { api, type App } from './api';

export function pollApp(id: string, intervalMs = 2000) {
  const [app, setApp] = createSignal<App | null>(null);
  async function tick() { setApp(await api.getApp(id)); }
  tick();
  const timer = setInterval(tick, intervalMs);
  onCleanup(() => clearInterval(timer));
  return app;
}
```

- [ ] **Step 5: Build the 4 screens**

Same screen set and behavior as Task 2 Step 5 / Task 3 Step 5, written
as Solid components (`createSignal`/`createResource` for the Builder's
pack list and app list, `pollApp` from Step 4 for the workflow rail,
`createResource` for gate report and audit trail). Route files follow
whichever router survived Step 1 (`@tanstack/solid-router` file-based
convention `apps.$id.tsx`, or `@solidjs/router`'s own convention if that
was the fallback — name the files to match whichever is actually wired
up, and say which in the self-report).

- [ ] **Step 6: Verify against the live API**

```bash
cargo run &
cd web-treatments/solid-tanstack && npm run dev
```
Same manual verification as Tasks 2/3 Step 6. Kill `cargo run` when done.

- [ ] **Step 7: Write the self-report**

`docs/treatments/ui-solid-tanstack.md`, same format, explicitly covering:
the Step 1 maturity finding (verbatim disclaimer or the fallback taken),
fine-grained-signal ergonomics vs. Svelte/Vue reactivity, TanStack Query
cache/loading-state ergonomics if used, bundle size observed vs. Tasks 2/3.

- [ ] **Step 8: Commit**

```bash
git add web-treatments/solid-tanstack docs/treatments/ui-solid-tanstack.md
git commit -m "treatment: Solid+TanStack Pareto screens against live API"
```

---

### Task 5: Decide the winner

**Files:**
- Create: `docs/treatments/ui-framework-comparison.md`

**Interfaces:**
- Consumes: all three self-reports (Tasks 2–4, Step 7) and each
  treatment's `git diff --stat` footprint.
- Produces: a named winner (`svelte` | `nuxt` | `solid-tanstack`) that
  Task 6 builds against, and a primitives list Task 6 must carry over
  (API client shape, polling pattern, token consumption, screen
  breakdown) regardless of which framework won.

- [ ] **Step 1: Score each treatment**

Table with rows = {SvelteKit, Nuxt, Solid+TanStack}, columns =
{ecosystem maturity, primitive/helper reusability, time-to-working-
screen (from Step 6 timestamps in each treatment task), ease of
debugging (from each self-report's "what fought the framework"),
Solid's maturity risk (Task 4 Step 1 finding) as an explicit
disqualifying-or-not column}. Fill every cell from the self-reports —
no cell may say "TBD" or be left blank.

- [ ] **Step 2: State the winner and reasons**

One paragraph: which framework, and why, referencing the score table
directly (e.g. "Nuxt wins on ecosystem maturity and time-to-working-
screen; Solid+TanStack's router required the fallback noted in its
self-report, which disqualifies it as an ecosystem-maturity outlier per
the design's explicit risk flag").

- [ ] **Step 3: List the extracted primitives/helpers**

Regardless of winner: the API client contract (Task 2 Step 3's
interface, which all three treatments implemented identically), the
polling pattern (three different implementations of the same idea —
note which one Task 6 will port), the token-consumption pattern
(`@import` + `var(--st-*)`), the 4-screen decomposition itself. This
section is what later becomes the "primitives extracted" sub-issue
(Task 9).

- [ ] **Step 4: Document component-design problems found**

Concrete, sourced from the self-reports: e.g. "all three treatments
independently re-implemented gate-check pass/fail styling — that
should be a shared primitive before Task 6, not per-screen inline
styles" or "polling via `setInterval` fights SSR in Nuxt (Step 6
observation) — the winner's rollout should use its native data-
fetching primitive instead if it's not Nuxt, or resolve the SSR guard
properly if it is."

- [ ] **Step 5: Commit**

```bash
git add docs/treatments/ui-framework-comparison.md
git commit -m "docs: score treatments, decide framework, extract primitives"
```

---

### Task 6: Full rollout — replace web/index.html with the winner

**Files:**
- Modify: `web/` (becomes the winning framework's project root, or a
  `web/app/` subfolder with `web/index.html` as its single-file build
  output — pick whichever keeps `src/api.rs`'s
  `include_str!("../web/index.html")` valid without touching Rust code;
  document the choice at the top of `web/README.md`)
- Create: `web/README.md` (how to run dev, how to build, how the single-
  file output is produced)
- Modify: `src/api.rs:41` only if the served file's path changes
- Test: manual verification against `cargo run`, see Step 5

**Interfaces:**
- Consumes: Task 5's winner + primitives list; the winning treatment's
  own code from Task 2/3/4 as the starting point (move/rename its
  directory into `web/`, don't rewrite from scratch).

- [ ] **Step 1: Rebase onto current main**

```bash
git fetch origin main
git rebase origin/main
```
Resolve any conflicts (there should be none — `web/index.html` is the
only file both this branch and `main` could plausibly touch; if `main`
changed it, take `main`'s version as the pre-rollout baseline and reapply
this task on top of it).

- [ ] **Step 2: Promote the winning treatment into `web/`**

```bash
git mv web-treatments/<winner>/* web/
git rm web/index.html   # the old vanilla file, once its replacement builds (Step 4)
```

- [ ] **Step 3: Extend to full 8-stage parity**

Add the 4 stages the Pareto screens didn't cover — `deploy` and
`operate` views (GET `/api/apps/:id/operate`, `/api/apps/:id/
operations`), plus `restore`/`review` actions (POST `/api/apps/:id/
restore`, `/api/apps/:id/review`) surfaced on the workflow-rail screen
next to the existing iterate/promote/rollback controls, and an export
link (GET `/api/apps/:id/export`, GET `/api/audit/export`) on the audit
screen. Reuse the Step-4/Task-5 API client and polling primitive; add
the new endpoints to the same `api.ts`/`useApi.ts` file rather than a
new one.

- [ ] **Step 4: Configure the single-file production build**

Add `vite-plugin-singlefile` to the winner's Vite config (already
installed in Tasks 2–4), set the build output to emit exactly
`web/index.html` with all JS/CSS inlined:

```ts
import { viteSingleFile } from 'vite-plugin-singlefile';
// in defineConfig plugins array, alongside the framework plugin:
plugins: [/* framework plugin */, viteSingleFile()],
build: { outDir: '.', emptyOutDir: false }
```

Adjust `outDir`/build target per framework (SvelteKit and Nuxt both
need their SSR/adapter config set to static/SPA output first — e.g.
`@sveltejs/adapter-static` or Nuxt's `nitro.preset = 'static'` — before
`vite-plugin-singlefile` can inline a single HTML file; do this
adapter/preset change as part of this step, not a follow-up).

```bash
cd web && npm run build
```

- [ ] **Step 5: Verify the built artifact serves from the real binary**

```bash
cd /Users/jaybhagat/Documents/qedc/hashistack-healthcare
cargo run
```
In a browser, open `http://127.0.0.1:3000/` and walk the full workflow:
create an app, iterate, view gate report and fix a check, promote,
view operate/audit, export. Confirm no console errors and no requests
to a dev-proxy origin (everything must resolve against the same-origin
`/api/*` the Rust server serves, since there's no Vite proxy at runtime
— this is why Step 3's `api.ts` must use relative `/api/...` paths, not
`http://127.0.0.1:3000/api/...`).

- [ ] **Step 6: Commit**

```bash
git add web/ src/api.rs
git commit -m "feat: replace web/index.html with <winner>-built doctor UI"
```

---

### Task 7: Screenshots

**Files:**
- Create: `docs/treatments/screenshots/` (PNG files, before/after per
  stage)

**Interfaces:**
- Consumes: `cargo run` serving the new `web/index.html` (Task 6); a
  second checkout of the pre-rollout `web/index.html` (via `git show
  origin/main:web/index.html`) for the "before" set.

- [ ] **Step 1: Capture "after" screenshots**

With `cargo run` serving the Task 6 build, use Chrome browser automation
(load `mcp__claude-in-chrome__*` tools via ToolSearch first) to navigate
`http://127.0.0.1:3000/` and screenshot each of the 8 workflow stages
plus the gate-report and audit-trail screens. Save as
`docs/treatments/screenshots/after-{stage}.png` for each of: builder,
generate, preview, iterate, gate, deploy, operate, audit.

- [ ] **Step 2: Capture "before" screenshots**

Write `origin/main`'s `web/index.html` to a scratch file, serve it
statically (e.g. `python3 -m http.server` from a temp dir containing
just that file — note in the issue that it won't have live API data
since it's the standalone file, so these are UI/UX-only captures), and
screenshot the same set of stages/skins the file exposes via its
`.skins` toggle. Save as `docs/treatments/screenshots/before-{stage}.png`.

- [ ] **Step 3: Commit**

```bash
git add docs/treatments/screenshots/
git commit -m "docs: before/after screenshots for the UI framework rollout"
```

---

### Task 8: Push, open the draft PR, close it

**Files:** none (GitHub operations only)

- [ ] **Step 1: Push the branch**

```bash
git push -u origin claude/treatment-planning-ui-frameworks
```

- [ ] **Step 2: Open the draft PR**

```bash
gh pr create --draft --title "Exploratory: doctor-UI framework treatments (Svelte / Nuxt / Solid+TanStack)" --body "$(cat <<'EOF'
## Summary
Three throwaway Pareto-screen treatments against the live API, scored in
docs/treatments/ui-framework-comparison.md, winner rebuilt to full
8-stage parity and made the served UI in web/. Evidence, not a
deliverable — per docs/process/gitops-treatments.md, this PR is closed
without merging; see the tracking issue for what actually ships.

## Self-reports
- docs/treatments/ui-svelte.md
- docs/treatments/ui-nuxt.md
- docs/treatments/ui-solid-tanstack.md
- docs/treatments/ui-framework-comparison.md (decision + primitives)

## Screenshots
docs/treatments/screenshots/ (before/after, all 8 stages)
EOF
)"
```

- [ ] **Step 3: Close the draft PR**

```bash
gh pr close claude/treatment-planning-ui-frameworks --comment "Closing per the treatments ritual — evidence preserved on this branch and in docs/treatments/. Tracked in issue #<parent-issue-number> (Task 9)."
```
(Fill `<parent-issue-number>` after Task 9 Step 1 creates it — do Task 9
Step 1 first if running these in order, or come back and edit the close
comment via `gh pr comment` once the issue number is known.)

---

### Task 9: File the parent issue + sub-issues

**Files:** none (GitHub operations only)

- [ ] **Step 1: Create the parent issue**

```bash
gh issue create --title "UI: replace web/index.html with <winner> — enable + why the current UX isn't production-ready" --label "area:web,type:feature" --body "$(cat <<'EOF'
Closes out the exploratory PR #<pr-number> (Svelte/Nuxt/Solid+TanStack
treatments, docs/treatments/ui-framework-comparison.md).

## What's enabled
web/ now builds a <winner> app to a single-file web/index.html (vite-plugin-singlefile), served unchanged by src/api.rs's include_str!. Run: `cd web && npm run build && cd .. && cargo run`.

## Why web/index.html (pre-rollout) wasn't production-ready
- Single 551-line file, inline styles, no component reuse
- No loading/error states beyond hand-rolled ad hoc handling
- No accessibility audit (focus management, ARIA on the modal/drawer patterns)
- No responsive behavior below its fixed max-width layout
- No tests

## Evidence
docs/treatments/screenshots/{before,after}-*.png (all 8 stages)
docs/treatments/ui-{svelte,nuxt,solid-tanstack}.md (self-reports)
docs/treatments/ui-framework-comparison.md (decision + primitives)
EOF
)"
```

- [ ] **Step 2: Create sub-issues, each referencing the parent**

```bash
gh issue create --title "UI framework treatment: SvelteKit — best practices + proof" --body "Self-report: docs/treatments/ui-svelte.md. Sub-issue of #<parent>."
gh issue create --title "UI framework treatment: Nuxt — best practices + proof" --body "Self-report: docs/treatments/ui-nuxt.md. Sub-issue of #<parent>."
gh issue create --title "UI framework treatment: Solid+TanStack — best practices + proof" --body "Self-report: docs/treatments/ui-solid-tanstack.md. Sub-issue of #<parent>."
gh issue create --title "Primitives/helpers extracted from the UI treatments" --body "docs/treatments/ui-framework-comparison.md, Step 3. Sub-issue of #<parent>."
```

- [ ] **Step 3: Link sub-issues back into the parent**

```bash
gh issue edit <parent> --body "$(gh issue view <parent> --json body -q .body)

## Sub-issues
- #<svelte-issue>
- #<nuxt-issue>
- #<solid-issue>
- #<primitives-issue>"
```

---

## Self-review notes

- **Spec coverage:** all 9 steps of the design doc map 1:1 to Tasks 1–9.
- **Placeholder scan:** `<winner>`, `<pr-number>`, `<parent>`, `<*-issue>`
  are intentional runtime substitutions (values don't exist until prior
  steps run), not unresolved-content placeholders — every step around
  them has real, runnable content.
- **Type consistency:** `Pack`/`App`/`GateReport`/`AuditEntry` and the
  `api.ts`/`useApi.ts` function names (`listPacks`, `listApps`,
  `createApp`, `getApp`, `iterate`, `gateReport`, `fixGate`, `promote`,
  `rollback`, `audit`) are identical across Tasks 2–4 and referenced
  unchanged in Task 6.
