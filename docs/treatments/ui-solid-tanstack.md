# Treatment report: Solid + TanStack Router

## Maturity finding (read this first)

The brief's premise — "Solid support in TanStack Start/Router is
experimental" — did **not** hold up against the actual, currently
published package. Checked directly against the npm registry before
writing any code:

```
$ npm view @tanstack/solid-router versions --json | tail
  "2.0.0-alpha.9", "2.0.0-beta.10" ... "2.0.0-beta.23"
$ curl -s https://registry.npmjs.org/@tanstack/solid-router | jq '."dist-tags"'
{ "alpha": "2.0.0-alpha.9", "beta": "2.0.0-beta.23", "latest": "1.170.17" }
```

`@tanstack/solid-router`'s `latest` npm dist-tag points at **`1.170.17`**
— a stable, semantic-release-tagged 1.x line (the README's own badge
reads `semantic-release 🚀`), not the alpha/beta 2.0 line. Its
description is plainly "Modern and scalable routing for Solid
applications" with no experimental/alpha disclaimer anywhere in the
published README (`packages/solid-router/README.md` on the `main`
branch — fetched and read in full; it contains marketing badges and a
link to the docs site, nothing under headings like "Status" or
"Stability"). `@tanstack/solid-start` likewise has a stable `latest`
dist-tag (`1.168.27`), separate from its own 2.0 alpha/beta line. `npm
install @tanstack/solid-router@^1 @tanstack/solid-query` resolved
cleanly with **zero peer-dependency warnings** against `solid-js@1.9.14`
and Vite 8, and `@tanstack/router-plugin`'s config type literally
enumerates `target: "vue" | "react" | "solid"` as a first-class,
symmetric option alongside React and Vue — not a bolt-on.

**No fallback to `@solidjs/router` was needed.** This task used
`@tanstack/solid-router@^1.170.17` (file-based routing via
`@tanstack/router-plugin/vite`) exactly as the brief's Step 1 primary
path describes, and it worked end-to-end against the live API with no
router-specific runtime errors. The only real defect found (see "what
fought the framework" below) was a routing *design* mistake on my part
— nested vs. flat routes — not a maturity problem with the package
itself. If there is a legitimate maturity gap in TanStack's Solid
support today, it's more likely in less-visited surface area (SSR/Start
streaming, devtools polish) that this Pareto-scope, client-only SPA
never exercised — that caveat is worth weighing in Task 5's scoring,
but "the package is broken or unpublished" specifically is **not**
accurate as of this session (2026-07-13).

## Design in five sentences

Everything reactive is a `createSignal`/`createResource` call, and
unlike Svelte's compiler-rewritten `$state` or Vue's `.value`-boxed
`ref`, a Solid signal is literally a getter function you call (`app()`,
not `app` or `app.value`) — this reads slightly more verbose at every
call site but means "is this reactive?" is always answerable by looking
at whether there are trailing parens, with zero compiler magic involved.
Routing is file-based under `src/routes/**` exactly like SvelteKit's
`routes/` and Nuxt's `app/pages/`, so the four required screens are
again just the four files the brief asked for — but TanStack Router's
file convention encodes route *nesting* into the filename itself
(`apps.$id.gate.tsx` nests under `apps.$id.tsx` by default), which is a
real behavioral difference from SvelteKit/Nuxt's directory-based routing
where sibling pages never implicitly become parent/child. There is no
virtual DOM and no SSR to reason about at all in this treatment — this
is a plain client-rendered Vite SPA (no TanStack Start), so none of
Task 2's SSR-vs-`onMount` pitfall or Task 3's Nuxt-4 `srcDir` scaffold
drift applied here; the closest analogue turned out to be router
nesting semantics instead (see below). `createResource`'s ergonomics
for the Builder's pack/app lists and the gate/audit screens were close
to Svelte's `onMount`+`$state` in line count, with the advantage that
`refetch()` is handed back directly from the resource tuple instead of
needing a hand-rolled reload function. `@tanstack/solid-query` was
installed per the brief's Step 1 but never actually used in the four
screens — Solid's built-in `createResource` covered every data-fetching
need at this Pareto scope (no need for cross-component cache sharing,
background refetch-on-focus, or query invalidation), so it's dead
weight in `package.json` for this specific app, not a verdict on
solid-query's own maturity (which, like TanStack Query generally, is
long-stable).

## Footprint

- **Files (excluding `node_modules`/`.tanstack`/`dist`):** 22 files.
  Hand-written application code is 718 lines across `src/lib/api.ts`
  (144, a direct copy of Task 2's contract), `src/lib/stages.ts` (26),
  `src/lib/pollApp.ts` (22, matches the brief's Step 4 snippet nearly
  verbatim), `src/app.css` (84, importing Task 1's `tokens.css`),
  `src/main.tsx` (16, router setup), `src/routes/__root.tsx` (14), and 4
  route files (`index.tsx` 119, `apps.$id.tsx` 149,
  `apps.$id_.gate.tsx` 87, `apps.$id_.audit.tsx` 57).
- **`package.json` dependency count:** 3 runtime
  (`@tanstack/solid-router`, `@tanstack/solid-query`, `solid-js`), 4 dev
  (`@tanstack/router-plugin`, `@types/node`, `typescript`,
  `vite-plugin-solid`, `vite`) — comparable in shape to Svelte's
  all-devDependency profile, except Solid's router/query genuinely ship
  to the client as runtime deps (there's no compile-away step for
  routing the way Svelte compiles markup away).
- **`npm install` tree:** 73 top-level packages under `node_modules` —
  between SvelteKit's 57 and Nuxt's ~600; closer to SvelteKit's end
  since this is a plain SPA with no server engine/Nitro equivalent.
- **Production build output (`npm run build`, gzip):** 164 KB total
  uncompressed / ~46 KB gzipped across all chunks, with real per-route
  code-splitting — `apps.$id_.gate` and `apps.$id_.audit` each ship as
  their own ~0.3-2.4 KB chunk, loaded only when navigated to (visible in
  the `vite build` chunk list: `apps._id_.gate-*.js`,
  `apps._id_.audit-*.js` separate from the `index-*.js` main bundle).
  The largest chunks are `index-*.js` (54.5 KB / 17.6 KB gzip, app code
  + solid-js runtime) and `link-*.js` (48 KB / 17.7 KB gzip, TanStack
  Router's `<Link>`/matching machinery) — router code is a comparable
  size to the entire rest of the app, which is worth noting since
  neither Tasks 2 nor 3 produced a `dist/` build to compare gzip numbers
  against directly (their reports don't include this metric).
- **Before running `npm run dev`:** copy `.env.example` to `.env` (sets
  `VITE_DEV_TOKEN`) — `src/lib/api.ts` throws at import time if unset,
  matching Task 2/3's fail-loud requirement exactly.
- **Dev-server startup time observed in Step 6:** `VITE v8.1.4 ready in
  447 ms` — comparable to SvelteKit's ~450-600ms and faster than Nuxt's
  full Nitro+Vite startup, unsurprising since this treatment has no
  server-rendering engine to boot at all.

## What worked / what fought the framework

**Worked:**
- `@tanstack/router-plugin`'s `target: 'solid'` code-generation was
  exactly as advertised: editing route files under `src/routes/`
  regenerated `src/routeTree.gen.ts` on save via Vite's dev-server
  watcher, with full type-safety end to end — `Link to="/apps/$id/gate"
  params={{ id }}` is a compile error if the `to` string doesn't match a
  real route or the `params` shape doesn't match that route's dynamic
  segments. Neither SvelteKit's `href="/apps/${id}/gate"` nor Nuxt's
  equivalent gets this for free; it's a genuine, meaningfully different
  ergonomic advantage TanStack Router has over string-based routing.
- `createResource` + `<For>`/`<Show>` for the Builder, gate report, and
  audit trail screens (list rendering, conditional loading/error states)
  were about as compact as Svelte's `{#each}`/`{#if}` or Vue's
  `v-for`/`v-if` — no meaningful ergonomics gap for this class of UI.
- `pollApp`'s `createSignal` + `setInterval` + `onCleanup` (straight from
  the brief's Step 4 snippet) worked exactly as written on the first
  try, including cleanup firing correctly when navigating away from the
  workflow rail mid-poll (verified: no lingering network requests after
  navigating Rail → Builder in the browser's Network panel).
- TypeScript inference through `api.ts`'s typed `request<T>()` flowed
  cleanly into every `createSignal<App | null>()` / `createResource(() =>
  api.gateReport(id))` call site, same as both prior treatments.
- Promote → poll picked up `stage: "live"` and the rail heuristic
  (`live → 6` / `sandbox → 3`) advanced correctly within one 2-second
  tick, with zero manual refresh — confirms Solid's signal-driven
  re-render is exactly as fine-grained as advertised: only the rail's
  `<span>` `classList` and the `pack/stage/version` text node
  re-rendered, not the whole component tree (spot-checked via the
  browser's React/Solid-devtools-style highlight-on-update; no visible
  repaint flash on the Iterate form or the Promote/Rollback buttons
  during a poll tick).

**Fought the framework — one real bug, caught before commit:**
- TanStack Router's file-based convention treats a dot-separated prefix
  as *route nesting*, not just URL nesting. `apps.$id.gate.tsx` and
  `apps.$id.audit.tsx` were initially named to mirror Svelte/Nuxt's
  directory nesting (`apps/[id]/gate`, `apps/[id]/gate.vue`) — but in
  those frameworks nesting is purely a filesystem-path convenience with
  no implied parent/child *component* relationship (no shared
  `<Outlet/>` needed unless a `+layout.svelte`/layout file exists).
  TanStack Router is different: `apps.$id.tsx` automatically became the
  **layout route** for `apps.$id.gate.tsx`, and since `apps.$id.tsx`'s
  component has no `<Outlet />`, navigating to `/apps/:id/gate` changed
  the URL and matched the route (confirmed via `Route.useParams()`/dev
  console) but rendered **the parent's own JSX unchanged** — the gate
  report never appeared, silently, with no error. Root-caused by
  inspecting the generated `src/routeTree.gen.ts`
  (`AppsIdGateRoute.getParentRoute: () => AppsIdRoute`, not
  `rootRouteImport`) and confirmed against
  `@tanstack/router-generator`'s own type declarations
  (`node_modules/@tanstack/router-generator/dist/esm/utils.d.ts`, "trailing
  underscore" escape-from-nesting docs). Fixed by renaming the two
  routes to `apps.$id_.gate.tsx` / `apps.$id_.audit.tsx` — the trailing
  underscore on the `$id_` segment is TanStack Router's documented
  "flat route" escape hatch: it keeps the URL path
  (`/apps/$id/gate`/`/apps/$id/audit`) but makes both routes siblings of
  root instead of children of `apps.$id.tsx`, matching the sibling-page
  behavior Svelte/Nuxt gave for free. This is the one place TanStack
  Router's file convention actively surprised me relative to the other
  two treatments' router-file naming — worth flagging for Task 5 as a
  Solid/TanStack-specific footgun (nesting-by-filename, not
  nesting-by-directory, and silent on mismatch rather than erroring).

**API-contract mismatch (already known from Tasks 2 and 3, confirmed
identical a third time):** the brief's illustrative `api.ts`/
`GateReport`/`AuditEntry` shapes (`pack_id`, `state` as one of 8 stage
names, `checks[].passed`, flat audit array) do not match the real,
running API. This treatment ports Task 2's `src/lib/api.ts`/
`stages.ts` byte-for-byte (import syntax only differs), confirmed
against live responses in Step 6 below — no new divergence found on a
third independent read of the real contract.

## Step 6 verification notes

`cargo run` (from the main checkout,
`/Users/jaybhagat/Documents/qedc/hashistack-healthcare`, bound to
`127.0.0.1:3000` per `APP_BIND`'s default) reproduced the exact hang
both prior treatments documented: 0% CPU, no `rustc` child process,
empty stdout/stderr. Sampled the stuck process directly this time
(`sample <pid> 1`) and got the full stack trace confirming Task 2's
diagnosis precisely — the main thread was blocked in
`cargo::core::compiler::fingerprint::__compare_old_fingerprint` doing a
plain blocking `read()` syscall on the fingerprint cache
(`cargo_util::paths::read` → `std::fs::read` → `libsystem_kernel.dylib
read`), i.e. an environment-local sandboxed-filesystem stall, not a
code or dependency issue. Killed that specific `cargo run` PID (started
by this task, confirmed by PID before killing) and worked around it the
same way Task 3 did: ran the already-built binary directly
(`./target/debug/rust-proof-service`, built the same session, commit
`19317c4`) instead of going through `cargo run`'s build-freshness check
at all. This bound successfully and immediately (`control plane
listening on 127.0.0.1:3000`).

Verified via a real browser (`claude-in-chrome`), all against the live,
same-commit API:
- **Builder** (`/`): the real pack list (17 packs, starting with
  `compliance checklist`) rendered in the `<select>`; created an app
  (name defaulted to the pack) via `POST /api/apps` with the real
  `{ name?, pack, prompt }` body; it appeared in the Apps list
  immediately with `stage: sandbox`.
- **Workflow rail** (`/apps/compliance-checklist`): `pollApp` (2s
  interval) rendered `pack compliance-checklist · stage sandbox · v1`,
  rail highlighting "Iterate" (index 3) with describe/generate/preview
  marked done — identical heuristic and identical indices to Tasks 2/3.
- **Gate report** (`/apps/compliance-checklist/gate`, after the
  routing-nesting fix above): rendered `5/6 passed · 0 stubbed · not
  green`, with `auto-logoff after idle` showing `FAIL` and reason
  `auto-logoff after idle — not wired`; clicking **Fix** called `POST
  /apps/:id/gate/:gateId/fix` then re-fetched via `api.gateReport`,
  flipping the screen to `6/6 passed · 0 stubbed · green` live in the
  browser with no manual refresh.
- **Promote**: clicking Promote on the rail called `POST
  /apps/:id/promote` with `{ synthetic_demo: true }`; the poll picked up
  `stage: live, v2` within one 2-second tick and the rail advanced to
  "Operate" (index 6) — confirmed end-to-end through a real state
  transition, not just initial load.
- **Audit trail** (`/apps/compliance-checklist/audit`): rendered the
  real reverse-chronological event stream (`app.promoted`,
  `gate.passed`, `gate.fixed`, `agent.attempt`, `agent.scaffolded`,
  `agent.attempt`, `agent.routed`, ...) with `actor`/`detail`/
  ISO-formatted `at` timestamps, sorted by `seq` descending, including
  the co-sign/allocation detail text on `app.promoted`
  (`co-signed Dr. A. Osei ... allocation a-8718 in prod pool`).

Browser console showed 40 stale `[EXCEPTION] TypeError: Failed to
fetch` entries at page-load — these were all timestamped from an
earlier, unrelated SvelteKit dev-server session that had previously used
this same reused browser tab/port before this task started (stack
traces point at `svelte/src/lib/appStore.ts`, a file that doesn't exist
in this treatment); a fresh navigation + `read_console_messages` pass
produced zero new errors during any of the flows above. Killed only the
two processes this task started (`./target/debug/rust-proof-service` and
`npm run dev`, both by exact PID, confirmed dead via `ps -p` afterward)
— did not touch any other agent's processes or the other worktree.

## What I'd steal from the other treatments

Nuxt's auto-imports (no `import { ref } from 'vue'`/`import {
createSignal } from 'solid-js'` needed anywhere) would remove a real
sliver of boilerplate from every Solid route file here — Solid has no
equivalent, every primitive is an explicit import. SvelteKit and Nuxt's
purely filesystem-directory routing (no dot-segment nesting semantics to
reason about) is simpler to get right on the first try than TanStack
Router's file convention, which actively bit this treatment once (see
above) in a way neither prior treatment's router had a chance to.
