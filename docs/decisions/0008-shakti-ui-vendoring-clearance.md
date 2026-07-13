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
