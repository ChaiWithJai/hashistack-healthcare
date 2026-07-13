# Product use case

## Customer

The first user is a doctor who wants to make a small practice tool and learn
what it would take to make that tool safe. The doctor has a specific job in
mind but may not know how to structure an application.

The second user is the developer who receives the exported prototype. That
developer needs clear source, fixed tests, evidence of what the doctor
accepted, and a plain list of missing production controls.

## Job

The doctor wants to describe a practice problem, compare a small set of safe
changes, and see a useful synthetic preview. The doctor then wants to take the
source away so a developer can continue the work.

The workflow is:

```text
choose -> describe -> compare -> review -> check -> repair -> preview -> export
```

The doctor can complete the core workflow without login. Clerk asks for
identity only when the doctor claims or exports the workspace.

## Product boundary

Practice Studio creates a prototype with synthetic data. It helps the doctor
understand the job and helps a developer understand the accepted source. It
does not approve the application for patient data or clinical care.

The product must say what it proved and what remains. It must not turn a plan,
configuration file, or simulated check into a production claim.

## Rust owned risk

Rust owns the parts where an invalid answer could change the reviewed source
or misstate the evidence:

- Rust checks Gemma's bounded treatment response.
- Rust creates source from signed rules.
- Rust stores checkpoints and the accepted digest.
- Rust runs or requests fixed verification checks.
- Rust controls preview, export, and deployment decisions.
- Rust records the audit events that bind these steps together.

Gemma proposes a treatment. Gemma cannot use tools, read files, access secrets,
deploy code, or receive patient data.

## Transfer

The export is the transfer point. It contains an ordinary Svelte client, Rust
server, tests, synthetic fixtures, three editable diagrams, and one README.
The export does not require Practice Studio or one infrastructure vendor.

The developer can add patient data controls after choosing an architecture
that fits the practice. The reference guide may describe Nomad and Vault, but
the exported workflow and evidence format do not depend on them.
