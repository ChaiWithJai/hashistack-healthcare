# Framework-treatment findings — forward-looking research

This document indexes the UI framework treatments built during the research phase of this project. Three frameworks were independently evaluated against the live Rust API using the same four Pareto-critical screens (Builder, Workflow rail, Gate report, Audit trail). **This is research, not a shipped rollout.** While Tasks 1–5 were underway, the platform's actual doctor UI (`web/index.html`) gained real shipped functionality — Clerk authentication and a complete treatment-recipe flow (`describe → 3 signed recipes → diff review → gate → live`) — that none of the three treatments implement. Replacing `web/index.html` with any treatment's build would have regressed this shipped functionality. Per [Task 6's revised scope](../superpowers/plans/2026-07-12-ui-framework-treatment-and-rollout.md#task-6-revised-2026-07-13-rebase-only--no-rollout), the branch was rebased onto `main` without modifying the production doctor UI. The treatments stand as evidence for a possible future migration path.

## Treatment outcomes (one line each)

- **[SvelteKit](../treatments/ui-svelte.md)** — Leanest footprint (57 npm packages, 0 runtime deps) but hit an SSR crash (eager fetch killed the dev server) and required a security fix (hardcoded bearer token).
- **[Nuxt (Vue) — WINNER](../treatments/ui-nuxt.md)** — Largest ecosystem (~600 packages), zero rework cycles, SSR-safe by construction via `onMounted` semantics; best reusability via auto-imports and composables.
- **[Solid + TanStack Router](../treatments/ui-solid-tanstack.md)** — Stable ecosystem (maturity risk checked and disproven: `@tanstack/solid-router@1.170.17`), but thinnest ecosystem, unused `solid-query` dependency, and a silent router file-naming bug (`$id_` escape hatch required).

## Decision

**Nuxt (Vue) is the framework winner.** It reached a correct, reviewed implementation with zero defects and no rework cycle, while SvelteKit required two fixes (SSR crash, security) and Solid+TanStack required one (routing). Nuxt's `usePollApp` composable pattern — fetch inside `onMounted`, cleanup automatic via `onUnmounted` — is provably safe against the SSR crash class that took down SvelteKit's dev server. The framework's auto-imports and composable-first design gave it the best reusability score. Solid+TanStack is explicitly not disqualified on maturity, but its thinner ecosystem, client-shipped router cost (~17.7 KB gzip), and silent footgun (nesting-by-filename) place it third. SvelteKit's footprint advantage (leanest of the three) is outweighed by its two real defects — SSR crash and security violation — failure modes a healthcare-compliance product cannot tolerate as framework defaults.

## Details and primitives

See the detailed comparison document for scoring, extracted primitives/helpers (API client contract, polling pattern, token consumption, screen decomposition), and component-design problems found that should not repeat: [**UI Framework Comparison**](../treatments/ui-framework-comparison.md).
