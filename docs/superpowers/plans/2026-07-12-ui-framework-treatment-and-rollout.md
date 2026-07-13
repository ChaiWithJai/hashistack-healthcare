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

### Task 6 (REVISED 2026-07-13): Rebase only — no rollout

**Why revised:** while Tasks 1–5 ran, `origin/main` advanced 26 commits
past this branch's base, adding Clerk auth and a full "Shakti flow"
(describe → 3 signed treatment recipes → review diff → gate → live)
directly into `web/index.html`, plus Netlify/DigitalOcean/Cloudflare
delivery infra and Svelte-as-export-target code generation in
`src/eject.rs` (for clinicians' *exported* apps, not the platform's own
doctor UI). Replacing `web/index.html` with the Nuxt Pareto-screen build
would delete real, shipped functionality the treatments never
implemented. Per user direction (2026-07-13): **do not replace
`web/index.html`.** The three treatments and the comparison doc stand as
forward-looking research for a possible future migration, not something
landing now.

**Status: DONE.** The branch was rebased cleanly onto `origin/main`
(`2885c70`) with zero conflicts (`web-treatments/` and `docs/treatments/`
are new paths main never touched). A separate rollout attempt (promoting
`web-treatments/nuxt/` into `web/`, deleting `web/index.html`) was started
by an implementer subagent, recognized as wrong mid-flight, and fully
reverted (`git reset --hard` + `git clean`) before rebasing — no trace of
it remains on the branch.

Task 7 below is rewritten to cover the documentation the user actually
asked for instead: current-state doctor-UI journey, the exported
clinician deliverable, the delivery infra, and the framework-treatment
findings as one indexed package.

---

### Task 7 (REVISED 2026-07-13): Comprehensive documentation package

Four independent sub-tasks (7a–7d), each producing its own doc file(s)
under `docs/journey/`, run sequentially (not parallel — shared risk of
screenshot-directory/index-file collisions), followed by one index task
(7e) tying them together. Each sub-task is its own implementer + review
cycle, same as Tasks 1–5.

#### Task 7a: Doctor UI journey (current main)

**Files:**
- Create: `docs/journey/doctor-ui.md`
- Create: `docs/journey/screenshots/doctor-ui/` (PNG files)

**Interfaces:** consumes `cargo run` serving current `web/index.html`
(post-rebase, i.e. the real Shakti flow + Clerk + legacy skins).

- [ ] **Step 1:** Start `cargo run` from the main checkout (read-only;
  don't kill processes you didn't start). Load Chrome browser automation
  tools (`ToolSearch` for `mcp__claude-in-chrome__*`).
- [ ] **Step 2:** Walk and screenshot every screen/permutation reachable
  from `/`: the guest/anonymous describe flow, the Clerk sign-in
  ownership gate, the 3-recipe treatment comparison (`shaktiTreatments`),
  candidate review/diff (`shaktiCandidateReview`), the gate modal
  (`shaktiGate`), the live/post-launch dashboard (`shaktiLive`), and the
  four legacy skins if still reachable (Builder/Release path/Clinical
  view/Architecture). Name files by screen: `docs/journey/screenshots/
  doctor-ui/{describe,auth-gate,treatments,candidate-review,gate,live,
  skin-builder,skin-pipeline,skin-chart,skin-arch}.png`.
- [ ] **Step 3:** Write `docs/journey/doctor-ui.md`: one section per
  screen with its screenshot embedded, what triggers it, what state
  transition it represents (tie back to the `S` state machine fields
  found in `web/index.html`), and any rough edges observed (loading
  states, error handling, accessibility) — evidence-based, not
  speculative.
- [ ] **Step 4:** Commit: `git add docs/journey/doctor-ui.md docs/journey/screenshots/doctor-ui/ && git commit -m "docs: current doctor-UI journey with screenshots"`

#### Task 7b: Exported clinician deliverable

**Files:**
- Create: `docs/journey/exported-app.md`
- Create: `docs/journey/screenshots/exported-app/` (PNG files, reuse
  screenshots the generated `owned-app.mjs` Playwright test already
  produces under `web/test-results/` if an export is run, per Step 2)

**Interfaces:** consumes `src/eject.rs` (read-only) and one real export
run (`cargo run` + trigger export for `post-op-monitor`, or read the
generated file contents directly from `eject.rs`'s template functions if
running a live export isn't practical in this environment — say
explicitly in the doc which method was used).

- [ ] **Step 1:** Read `src/eject.rs` in full to understand exactly what
  gets generated (Svelte 5/SvelteKit source, `owned-app.mjs` Playwright
  journey test, README/CONTRIBUTING content, `.gitignore`/`.dockerignore`).
- [ ] **Step 2:** Either trigger a real export via the running API and
  inspect the generated tree, or (if that's not practical here) document
  precisely from `eject.rs`'s template strings what ships, clearly
  labeled as "read from generator source" vs. "observed from a live
  export."
- [ ] **Step 3:** Write `docs/journey/exported-app.md`: what a clinician
  receives (file tree, README/CONTRIBUTING excerpts), the post-op-monitor
  escalation flow specifically (patient submits pain score → queued →
  clinician inbox), the `owned-app.mjs` browser-journey test and what it
  asserts (including the zero-external-host-network-request check), and
  how a clinician would run/verify it themselves.
- [ ] **Step 4:** Commit: `git add docs/journey/exported-app.md docs/journey/screenshots/exported-app/ && git commit -m "docs: exported clinician deliverable"`

#### Task 7c: Frontend delivery infra

**Files:**
- Create: `docs/journey/delivery-infra.md`

**Interfaces:** consumes `netlify.toml`, `terraform/cloudflare/`,
`deploy/cloudflared/staging.yml.example`, `docs/decisions/0008-cloudflare-delivery-boundary.md`,
`docs/decisions/0009-agent-workspace-and-model-routing.md`,
`docs/digitalocean-runbook.md`, `.github/workflows/staging-preview.yml`,
`.github/workflows/cloudflare-dns.yml`, `verifier/` (all read-only).

- [ ] **Step 1:** Read every file listed above in full.
- [ ] **Step 2:** Write `docs/journey/delivery-infra.md` as one clear
  pipeline diagram-in-prose: what's actually live today (Netlify static
  host + proxy → hardcoded DO droplet IP) vs. what's staged-but-not-
  cut-over (Cloudflare DNS/tunnel terraform), the staging-preview GitHub
  Actions flow end to end (firewall punch → deploy exact PR SHA → comment
  URL back), and the `verifier/` sandboxed Docker verification pipeline
  (what it checks, why network is disabled). Be explicit about the gap
  between "accepted decision" (ADR 0008) and "what's actually wired" —
  don't present the Cloudflare path as live if it isn't.
- [ ] **Step 3:** Commit: `git add docs/journey/delivery-infra.md && git commit -m "docs: frontend delivery infra, current state vs. staged"`

#### Task 7d: Framework-treatment findings index

**Files:**
- Create: `docs/journey/framework-treatments.md` (a short pointer/summary
  doc — the actual findings already exist in `docs/treatments/*`, this
  task does not duplicate them)

**Interfaces:** consumes `docs/treatments/ui-svelte.md`,
`docs/treatments/ui-nuxt.md`, `docs/treatments/ui-solid-tanstack.md`,
`docs/treatments/ui-framework-comparison.md` (already committed, Tasks
2–5).

- [ ] **Step 1:** Write `docs/journey/framework-treatments.md`: one
  paragraph framing this as forward-looking research (not something that
  shipped — cross-reference Task 6's revision note above for why), a
  one-line summary of each treatment's outcome, the winner and why, and
  a link to each underlying doc. Keep this short; it's an index, not a
  restatement.
- [ ] **Step 2:** Commit: `git add docs/journey/framework-treatments.md && git commit -m "docs: index the framework-treatment findings as forward-looking research"`

#### Task 7e: Journey index

**Files:**
- Create: `docs/journey/README.md`

- [ ] **Step 1:** Write a short index linking all four docs (7a–7d) plus
  a one-paragraph statement of scope: this package documents UX, exported
  deliverable, delivery infra, and framework research as of the date
  written — a snapshot, not a living doc guaranteed to track future
  changes.
- [ ] **Step 2:** Commit: `git add docs/journey/README.md && git commit -m "docs: journey documentation package index"`

---

### Task 8: Push, open the draft PR, close it

**Files:** none (GitHub operations only)

- [ ] **Step 1: Push the branch**

```bash
git push -u origin claude/treatment-planning-ui-frameworks
```

- [ ] **Step 2: Open the draft PR**

```bash
gh pr create --draft --title "Exploratory: doctor-UI framework treatments + journey documentation" --body "$(cat <<'EOF'
## Summary
Three throwaway Pareto-screen treatments (SvelteKit/Nuxt/Solid+TanStack)
against the live API, scored in docs/treatments/ui-framework-comparison.md.
Nuxt won on ecosystem maturity, reusability, and time-to-working-screen.
This is forward-looking research, NOT a rollout: web/index.html is
unchanged (it has since gained Clerk auth and a full treatment-recipe
flow on main that none of the treatments implement — replacing it would
have regressed shipped functionality). Evidence, not a deliverable — per
docs/process/gitops-treatments.md, this PR is closed without merging; see
the tracking issue for the actual documentation deliverable.

Also includes docs/journey/ — a documentation package covering the
current doctor-UI journey, the exported clinician deliverable, and the
frontend delivery infra, with screenshots.

## Self-reports (framework treatments)
- docs/treatments/ui-svelte.md
- docs/treatments/ui-nuxt.md
- docs/treatments/ui-solid-tanstack.md
- docs/treatments/ui-framework-comparison.md (decision + primitives)

## Journey documentation
- docs/journey/doctor-ui.md (+ screenshots)
- docs/journey/exported-app.md (+ screenshots)
- docs/journey/delivery-infra.md
- docs/journey/framework-treatments.md (index into the treatments above)
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
gh issue create --title "Journey documentation: doctor UX, exported deliverable, delivery infra, framework research" --label "area:web,type:docs" --body "$(cat <<'EOF'
Closes out the exploratory PR #<pr-number> (docs/journey/ + docs/treatments/).

## What this covers
- docs/journey/doctor-ui.md — every screen/permutation of the current
  doctor UI (Shakti flow + Clerk + legacy skins), with screenshots
- docs/journey/exported-app.md — what a clinician actually receives
  (generated Svelte+Rust app, browser-journey test, per-pack flow)
- docs/journey/delivery-infra.md — the Netlify/DigitalOcean/Cloudflare
  delivery pipeline, live vs. staged-but-not-cut-over
- docs/journey/framework-treatments.md — index into the SvelteKit/Nuxt/
  Solid+TanStack research (docs/treatments/), forward-looking, not shipped

## Why this instead of a UI rollout
web/index.html gained Clerk auth and a full treatment-recipe flow while
the framework treatments were in progress; replacing it with any
treatment's build would have regressed shipped functionality. The
treatments stand as research for a possible future migration.

## Evidence
docs/journey/screenshots/{doctor-ui,exported-app}/*.png
docs/treatments/ui-{svelte,nuxt,solid-tanstack}.md (self-reports)
docs/treatments/ui-framework-comparison.md (decision + primitives)
EOF
)"
```

- [ ] **Step 2: Create sub-issues, each referencing the parent**

```bash
gh issue create --title "Journey doc: doctor UI UX audit + screenshots" --body "docs/journey/doctor-ui.md. Sub-issue of #<parent>."
gh issue create --title "Journey doc: exported clinician deliverable" --body "docs/journey/exported-app.md. Sub-issue of #<parent>."
gh issue create --title "Journey doc: frontend delivery infra (Netlify/DO/Cloudflare)" --body "docs/journey/delivery-infra.md. Sub-issue of #<parent>."
gh issue create --title "UI framework treatment: SvelteKit — best practices + proof" --body "Self-report: docs/treatments/ui-svelte.md. Sub-issue of #<parent>."
gh issue create --title "UI framework treatment: Nuxt — best practices + proof (winner)" --body "Self-report: docs/treatments/ui-nuxt.md. Sub-issue of #<parent>."
gh issue create --title "UI framework treatment: Solid+TanStack — best practices + proof" --body "Self-report: docs/treatments/ui-solid-tanstack.md. Sub-issue of #<parent>."
gh issue create --title "Primitives/helpers extracted from the UI treatments" --body "docs/treatments/ui-framework-comparison.md, Step 3. Sub-issue of #<parent>."
```

- [ ] **Step 3: Link sub-issues back into the parent**

```bash
gh issue edit <parent> --body "$(gh issue view <parent> --json body -q .body)

## Sub-issues
- #<doctor-ui-doc-issue>
- #<exported-app-doc-issue>
- #<delivery-infra-doc-issue>
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
