# UI framework comparison — deciding the winner

This document is the architectural decision point (Task 5) for the
HashiStack healthcare studio UI. Three framework treatments were built
independently against the **same** real Rust API, the **same** four
Pareto screens (Builder, Workflow rail, Gate report, Audit trail), and
the **same** corrected API contract. All three were reviewed and
approved. This document scores them, names a winner Task 6 will build
against, and extracts the primitives Task 6 must carry over regardless
of which framework won.

Sources: the three self-reports (`ui-svelte.md`, `ui-nuxt.md`,
`ui-solid-tanstack.md`), the review history for Tasks 2–4, and direct
spot-checks of the treatment code (`web-treatments/{svelte,nuxt,solid-tanstack}/`).

---

## Step 1 — Score table

Scores are `strong` / `good` / `adequate` / `weak`, each backed by a
sourced fact. No cell is speculative.

| Framework | Ecosystem maturity | Primitive/helper reusability | Time-to-working-screen | Ease of debugging | Solid maturity risk (disqualifying?) |
|---|---|---|---|---|---|
| **SvelteKit** | **good** — SvelteKit + Svelte compiler are stable and widely used; leanest footprint of the three (8 devDeps, **0 runtime deps**, 57 npm packages, no client-shipped router). | **good** — typed `request<T>()` API client + `writable` store polling primitive; but store needs a manual `.stop()` contract and an explicit `import { onMount }` on every route (no auto-import). | **good** — dev server `ready in 406 ms` cold / 607 ms warm. But real time-to-*correct*-screen was hurt by **two rework cycles**: an SSR crash and a hardcoded-token security fix (see debugging). | **weak** — the SSR failure mode was the worst of the three: an eager top-level `fetch` with a relative URL threw an uncaught exception that **killed the whole Node dev-server process**, silently, with no compiler/dev-server warning until the request landed. | N/A — not Solid. |
| **Nuxt (Vue)** | **strong** — largest ecosystem of the three (Vue + Nitro + the Nuxt module system); everything used is on a stable `latest`. Cost: **heaviest install (~600 npm packages** vs. 57/73) — the one real mark against it. | **strong** — same typed `request<T>()` client, plus Nuxt **auto-imports** (`ref`, `computed`, `onMounted`, `useRoute`, and every `app/composables/*`) remove a whole class of import boilerplate; polling lives in a clean `usePollApp` composable. | **strong** — Nitro built in 273 ms / Vite client 15 ms, comparable dev startup; **zero review findings** and no rework cycle, so time-to-*correct*-screen was the best of the three. (Slower one-time `npm install` due to the large tree.) | **strong** — no crash to debug. `onMounted` never fires during SSR, so the polling fetch is **safe by construction** — the SvelteKit crash class simply cannot occur. Remaining friction was *loud, explicit* errors (route 404s from Nuxt-4 `app/` `srcDir` drift; a `vue-tsc`/`typescript@7` `ERR_PACKAGE_PATH_NOT_EXPORTED`), which are far easier to diagnose than a silent process death. | N/A — not Solid. |
| **Solid + TanStack Router** | **adequate** — the package is **stable, not experimental**: `@tanstack/solid-router` `latest` dist-tag = `1.170.17` (semantic-release 1.x line), `target: "solid"` is a first-class enum in `@tanstack/router-plugin`, clean install with zero peer warnings. But it is the **thinnest ecosystem** of the three, and `@tanstack/solid-query` shipped as an installed-but-**unused** runtime dep (dead weight for this scope). | **good** — same typed `request<T>()` client; `createResource` hands back `refetch()` for free (nicer than Svelte's hand-rolled reload), and TanStack Router gives **type-safe `<Link>`** (a compile error on a bad route/params — a genuine advantage). But every primitive is an explicit import (no auto-import), and the router is a real client-shipped runtime cost (`link-*.js` ~17.7 KB gzip, as large as the rest of the app). | **good** — dev server `ready in 447 ms`, 73 packages (between the other two). One rework cycle for the router-nesting bug, plus the up-front maturity-check overhead. | **adequate** — the router-nesting bug was **silent**: `apps.$id.gate.tsx` auto-nested under `apps.$id.tsx`, matched the URL but rendered the parent's JSX with **no error**. Root-caused only by reading the generated `routeTree.gen.ts`. Offset by type-safe links catching a class of errors at compile time. | **NOT disqualifying, but a scored negative.** The design's explicit risk flag ("Solid support in TanStack may be experimental") was **checked and disproven** — the package is stable and worked end-to-end against the live API with no router-specific runtime errors. However, it remains the thinnest ecosystem and produced a real *silent* footgun (nesting-by-filename) that neither directory-routed framework could hit. Stable enough to ship; not enough to win. |

---

## Step 2 — The winner

**Winner: Nuxt (Vue).** Nuxt wins on **ecosystem maturity** and
**ease of debugging**, ties for the best **time-to-working-screen**, and
takes **primitive/helper reusability** outright. The decisive factor is
that Nuxt reached a *correct, reviewed* implementation with **zero review
findings and no rework cycle**, while the two competitors each required
one: SvelteKit hit an SSR crash that took down the entire dev server
*and* shipped a hardcoded bearer-token literal a reviewer flagged as an
Important violation (two fix cycles); Solid+TanStack hit a silent
router-nesting bug. Nuxt's `onMounted` SSR semantics made the polling
fetch **safe by construction** — the exact failure class that crashed
SvelteKit's dev server cannot occur in this design — and its
auto-imports plus composable layout gave the best reusability scores in
the table. Solid+TanStack is explicitly **not disqualified** (its
maturity risk was checked and disproven: `latest` = `1.170.17`, stable,
no fallback to `@solidjs/router` needed), but as the score table shows it
is the thinnest ecosystem, carries a client-shipped router cost and an
unused `solid-query` dependency, and produced a *silent* footgun — enough
to keep it out of first place though not to rule it out. SvelteKit is the
honorable runner-up on footprint (the leanest of the three: 0 runtime
deps, 57 packages), but its two real defects — a silent whole-process
SSR crash and a caught security violation — are precisely the failure
modes a healthcare-compliance product cannot tolerate as framework
*defaults*, and they cost it the debugging column. Nuxt's single real
downside is the heaviest dependency tree (~600 packages), a
supply-chain/build-weight concern that does not touch runtime correctness
or security — an acceptable price for the safest defaults and the
cleanest reuse story.

---

## Step 3 — Extracted primitives / helpers (carry over regardless of winner)

These are framework-agnostic and become the "primitives extracted"
sub-issue (Task 9). Task 6 must carry all of them into `web/`.

### 1. API client contract (`api.ts` / `useApi.ts`)
All three treatments implemented the **identical corrected contract** —
the fact that three independent reads converged byte-for-byte is itself
evidence the contract is unambiguous once the Rust source is read
correctly. The canonical shape (typed `request<T>()` wrapper + fail-loud
on a missing dev bearer token):
- `AppRecord`: has `pack` (not `pack_id`) and `stage: "sandbox" | "live"`
  (not an 8-value `state`).
- `createApp({ prompt, pack, name? })` → returns `{ app, scaffold }`.
- `gateReport` → `{ report, meter, reviewer_note }`, with
  `results[].status: "pass" | "stubbed" | "fail"`.
- `audit` → `{ events: [...] }` with `seq` / `actor` / `action` /
  `detail` fields.
- Gate fix: `POST /apps/:id/gate/:gateId/fix`; promote:
  `POST /apps/:id/promote` with `{ synthetic_demo: true }`.
- The client **throws at construction/import time** if the dev bearer
  token env var is unset (fail-loud). **Note:** SvelteKit's first cut
  hardcoded the token literal — Task 6 must keep the token in
  env/runtime config (`NUXT_PUBLIC_DEV_TOKEN` for the Nuxt winner),
  never inline.

### 2. Polling pattern — three implementations, port the Nuxt one
Same idea (poll `GET /apps/:id` every 2 s so the Workflow rail picks up
server-driven stage transitions), three expressions:
- **Svelte** (`appStore.ts`): `writable` store returning
  `{ subscribe, stop }` — caller must remember to call `stop()`.
- **Nuxt** (`usePollApp.ts`): `ref` + `onMounted(tick + setInterval)` +
  `onUnmounted(clearInterval)`. **SSR-safe by construction** and
  self-cleaning.
- **Solid** (`pollApp.ts`): `createSignal` + `setInterval` +
  `onCleanup(clearInterval)` — also self-cleaning, verified no lingering
  requests after navigation.

**Task 6 ports the Nuxt `usePollApp` composable** (winner's native
idiom): the eager fetch sits inside `onMounted`, which never runs during
SSR, so the SvelteKit crash class is structurally impossible and cleanup
is automatic via `onUnmounted`. Do **not** reintroduce an eager
top-level `<script setup>` `await fetch(...)`.

### 3. Token-consumption pattern (design tokens)
`@import` of Task 1's `tokens.css` + consume via `var(--st-*)` custom
properties (e.g. `var(--st-success)`, `var(--st-danger)`). Ported 1:1
across all three treatments (`app.css` in each). Task 6 keeps this
verbatim.

### 4. The 4-screen decomposition
The Pareto screen set, each a single file-based route:
- **Builder** (`/`) — pack list + create-app form.
- **Workflow rail** (`/apps/:id`) — the polling screen; stage heuristic
  maps the real 2-value lifecycle (`sandbox`/`live`) onto 8 display
  labels (`sandbox → index 3` "Iterate", `live → index 6` "Operate").
  This stage-label heuristic (`stages.ts` / `useStages.ts`) is itself a
  shared primitive — identical indices across all three treatments.
- **Gate report** (`/apps/:id/gate`) — pass/stubbed/fail results + Fix
  affordance.
- **Audit trail** (`/apps/:id/audit`) — reverse-chronological event
  stream sorted by `seq` descending.

All three used file-based/directory routing, so the four screens are
four files with zero router config — **except** TanStack Router encodes
nesting into the filename (dot-segments), which required the `$id_` flat-
route escape hatch. Nuxt (the winner) uses plain directory routing under
`app/pages/**`, so this footgun does not carry forward.

---

## Step 4 — Component-design problems found (fix in Task 6, don't repeat)

Concrete issues, sourced from the reports and verified in the code:

1. **Gate pass/fail styling was independently re-implemented in every
   treatment — make it a shared primitive.** Verified directly: each
   gate screen (`svelte/.../gate/+page.svelte`, `nuxt/.../gate.vue`,
   `solid-tanstack/.../apps.$id_.gate.tsx`) hand-rolls its own near-
   identical `.card.pass` / `.card.fail` / `.status` CSS using
   `var(--st-success)` / `var(--st-danger)`. This is per-screen inline
   styling of a semantic that recurs (pass/fail/stubbed status also
   appears conceptually on the workflow rail and could on the audit
   trail). Task 6 should extract a single **`GateStatus` / status-pill
   primitive** (component + one styleblock keyed on `pass|stubbed|fail`)
   rather than copy the CSS into each screen. Note the current gate
   styling only branches on `pass`/`fail` and does not visually
   distinguish **`stubbed`** — the shared primitive should handle all
   three statuses the API actually returns.

2. **Polling must use the framework's SSR-safe data primitive, not an
   eager top-level fetch.** SvelteKit's crash came from an eager
   top-level `fetch` in page-script scope during SSR; it killed the dev
   server silently. For the Nuxt winner this is resolved by keeping the
   fetch inside `onMounted` (the `usePollApp` composable already does
   this correctly). Task 6's rollout must **not** regress to a top-level
   `await fetch('/api/...')` in `<script setup>`, and should not need
   `<ClientOnly>` if it follows the composable pattern.

3. **Stage-label heuristic is a display fiction — keep it labeled as
   such.** All three map the real 2-value `stage` onto 8 display labels.
   This is a UI heuristic, not an API field. Task 6 should keep it in one
   shared module (`useStages`) with a comment making clear the 8 labels
   are presentational, so a future contributor doesn't mistake them for
   real lifecycle states the server tracks.

4. **Drop dead dependencies on rollout.** The Solid treatment shipped
   `@tanstack/solid-query` unused. Independent of the winner, Task 6 must
   not carry installed-but-unused deps into `web/` — every runtime dep in
   the final `web/` should be one the four screens actually import.

---

## Known environment issues (not scoring factors)

- `cargo run` hung intermittently for **all three** treatments (0% CPU,
  blocked in `fingerprint::__compare_old_fingerprint` on a sandboxed-
  filesystem `read()`). This is environment-local, not framework-related;
  all three verified against an already-running same-commit
  (`19317c4`) binary instead. Not a differentiator.
- Verification for all three was done against the **live** API via real
  browser sessions (Playwright / claude-in-chrome) and `curl`, not mocks.
  All four screens round-tripped real requests end-to-end, including live
  state transitions (Fix flips gate to green; Promote advances the rail
  to `live`/v2).

---

## Decision summary

**Build Task 6 (`web/`, replacing `web/index.html`) with Nuxt (Vue).**
Carry over the four primitives in Step 3 (API client contract, the
**Nuxt `usePollApp`** polling composable, the `var(--st-*)` token
pattern, the 4-screen decomposition + stage heuristic). Fix the four
component-design problems in Step 4 during rollout — above all, extract a
shared pass/stubbed/fail status primitive instead of repeating per-screen
gate CSS.
