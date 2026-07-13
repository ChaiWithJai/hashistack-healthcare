# Shared design tokens

`tokens.css` is a hand-extracted set of CSS custom properties
(`--st-*` prefix) taken from the supplied design archive, per the written
vendoring clearance recorded in
[ADR 0008](../../docs/decisions/0008-shakti-ui-vendoring-clearance.md).

## Provenance

Source archive: `Lovable for clinicians on HashiStack.zip`, SHA-256 already
on record in [ADR 0007](../../docs/decisions/0007-shakti-design-system-provenance.md).
Values were read from that archive's `_ds/shakti-ui-*/_ds_manifest.json`
token list and `_ds_bundle.css` custom-property declarations (colors,
radii, spacing base unit, font stack, focus-ring treatment), then
transcribed by hand into `tokens.css`.

This file is **not** the archive's compiled `_ds_bundle.css`/`_ds_bundle.js`
and does not import or reference them. Those files, the archive's Catalyst
JSX component source, fonts, and demo assets are not vendored and stay out
of this repository, per ADR 0008's scope.

## Usage

All three framework treatments (`web-treatments/sveltekit`,
`web-treatments/nuxt`, `web-treatments/solid`) `@import "../tokens/tokens.css"`
and style exclusively with `var(--st-*)`. Never inline the archive's own
class names (e.g. `bg-rose-500`, `rounded-2xl`) or hex/oklch literals that
duplicate a token — if a value is missing, add it here first so all three
treatments (and the eventual `web/` rollout) stay in sync.
