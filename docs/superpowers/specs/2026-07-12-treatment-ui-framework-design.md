# Design: doctor-UI frontend framework treatment + decision

Date: 2026-07-12
Status: approved (pending final user sign-off on this file)

## Decision under test

Which frontend framework replaces `web/index.html` (currently vanilla
HTML/JS) for the doctor's full workflow —
`describe → generate → preview → iterate → gate → deploy → operate → audit` —
served by the existing Rust/axum API (`src/api.rs`)? React/Next are out
(explicit exclusion). Candidates: **SvelteKit**, **Nuxt (Vue)**,
**SolidJS + TanStack Start/Router**.

## Process

Unlike the repo's `docs/process/gitops-treatments.md` ritual (separate
branch/issue per candidate, judged only once staging exists), this round is
lighter-weight per direction: one branch off `main`, one PR, three
subfolders. It still borrows the ritual's self-report discipline and its
core rule — **a treatment branch is evidence, not a deliverable** — but it
adds a step the ritual doesn't have: **a decision is made now**, not
deferred to a later judge phase, based on three criteria: ecosystem
maturity, primitive/helper reusability, and ease of just getting the job
done (dev speed, escape hatches, debuggability).

## Steps

1. **Branch**: `claude/treatment-planning-ui-frameworks` off current `main`
   (not touching any `codex/*` branch — Codex is active on
   `codex/staging-preview-proof`).
2. **ADR 0008**: records the written clearance to vendor shakti-ui design
   tokens (supersedes ADR 0007's no-vendoring clause for tokens; component
   *code* stays independently authored per framework — the Catalyst JSX
   isn't portable to Svelte/Solid anyway).
3. **Three Pareto treatments**, each a small Vite-based app in
   `web-treatments/{svelte,nuxt,solid-tanstack}/`, dev-server-runnable
   against the real Rust API on `:3000` (bearer token from `env.example`,
   no mocked data). Each covers the same 4 Pareto-critical screens/flows,
   chosen because they exercise the axes frameworks actually differ on:
   - **Builder** — describe + pack-select form → POST `/api/apps` → list
     (forms, validation, list rendering)
   - **Workflow rail** — GET `/api/apps/:id` polled/reactive, stage
     indicator across preview/iterate/gate/deploy/operate (state machine +
     polling/reactivity model)
   - **Gate report** — GET `.../gate`, POST `.../fix` per failing check
     (nested reactive updates, optimistic UI, error states)
   - **Audit trail** — GET `.../audit`, export link (read-heavy list,
     pagination/virtualization if needed)
   Each treatment self-reports to `docs/treatments/ui-{svelte,nuxt,
   solid-tanstack}.md`: design in five sentences, footprint (`git diff
   --stat`, new deps, new config), what worked / fought the framework,
   what I'd steal from the others.
4. **Decide**: `docs/treatments/ui-framework-comparison.md` scores all
   three against ecosystem maturity, primitive reusability, and
   time-to-working-screen, states a winner with reasons, and lists the
   primitives/helpers (API client, polling/reactive-state wrapper, gate
   status component, token consumption pattern) extracted as the shared
   base regardless-of-winner.
5. **Build the whole thing** against the winner, targeting parity with
   `web/index.html`'s full workflow (all 8 stages, not just the 4 Pareto
   screens) inside `web/` (replacing `web/index.html` as the served UI;
   `src/api.rs`'s `doctor_ui` handler and Nomad service templates updated
   to serve the built app). Rebase the branch onto `main` if `main` moves
   before this lands.
6. **Screenshots**: capture the new UI (via Chrome/Playwright automation)
   at each of the 8 workflow stages, plus the current `web/index.html` at
   the same stages, for the issue's before/after evidence.
7. **Close the draft PR** (evidence, not merged raw — matches the ritual's
   rule) that carried the 3 Pareto treatments + comparison doc.
8. **File a GitHub issue** referencing the closed PR: what's enabled (the
   new full UI, how to run it), and why `web/index.html` today isn't
   production-ready from a UX standpoint (no loading/error states beyond
   what's hand-rolled, no accessibility audit, no responsive behavior,
   single 551-line file with inline styles, no component reuse, no tests) —
   with the before/after screenshots as evidence.
9. **Sub-issues**: one per framework treatment (best-practice write-up +
   proof), one for "primitives/helpers extracted," one for "roll out the
   winning framework to `web/` / retire `web/index.html`" (tracks step 5 as
   a follow-up if it isn't fully finished in this pass).

## Out of scope

- Auth beyond the existing bearer-token dev flow.
- Full pixel parity with the sketchy wireframe theme — the shakti-ui
  tokens replace it (that's the point of the vendoring clearance).
- Deploy/operate write actions beyond what's needed to render their state
  (promote/rollback buttons wired, not redesigned).
- Automated visual regression tooling — screenshots are for the issue,
  not a new CI gate.

## Risk called out explicitly

SolidJS + TanStack Start is the least mature candidate (experimental Solid
adapter). It stays in the comparison as a real data point — if it loses on
ecosystem maturity, that's a legitimate, expected outcome to document, not
a reason to have skipped it.
