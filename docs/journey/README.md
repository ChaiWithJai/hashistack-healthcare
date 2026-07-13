# Journey documentation

A snapshot, dated 2026-07-13, of the clinician platform's user-facing
surfaces and delivery infrastructure — not a living doc guaranteed to
track future changes. Covers UX, the exported deliverable, delivery
infra, and forward-looking framework research:

- [doctor-ui.md](doctor-ui.md) — every reachable screen/permutation of
  the current doctor UI (Clerk auth + the Shakti treatment-recipe flow +
  legacy skins), with screenshots.
- [exported-app.md](exported-app.md) — what a clinician actually
  receives on export: the generated Svelte+Rust app, its Playwright
  browser-journey test, and the post-op-monitor escalation flow.
- [delivery-infra.md](delivery-infra.md) — the Netlify/DigitalOcean/
  Cloudflare delivery pipeline, what's live today vs. staged-but-not-
  cut-over, and the `verifier/` sandbox.
- [framework-treatments.md](framework-treatments.md) — index into the
  SvelteKit/Nuxt/Solid+TanStack research (`docs/treatments/`); forward-
  looking, not shipped.
