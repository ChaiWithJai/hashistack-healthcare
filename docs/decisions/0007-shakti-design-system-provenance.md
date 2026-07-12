# ADR 0007: Project-owned warm clinician theme; supplied kit not vendored

Status: accepted

The supplied design archive is useful product research, but its compiled
Shakti kit says it is derived from Tailwind Catalyst and includes Tailwind
Plus terms that restrict redistribution and products that let customers build
their own products. This repository is both an open builder and an exporter,
so the archive's JavaScript, CSS, components, tokens, fonts, and demo assets
must not enter the repository, runtime, or generated bundles without separate
written clearance.

The warm clinician theme in this project is independently authored:
a small semantic token set, warm but high-contrast surfaces, 44-pixel targets,
visible keyboard focus, reduced-motion behavior, plain clinical language, and
the existing dependency-free API-backed state machine. Generated applications
remain project-owned and carry no Catalyst or archive code.

Archive reviewed: `Lovable for clinicians on HashiStack.zip`, SHA-256
`9644ef9463315c67ca8432d3f2c1431da2978999880125fd4d4c3bb714d018a1`.
