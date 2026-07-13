# RFC 0001: Practice Studio platform design

Status: accepted and amended by
[decision 0010](../decisions/0010-minimum-lovable-runtime.md)

Model policy: [decision 0009](../decisions/0009-agent-workspace-and-model-routing.md)

Design steering: [HashiCorp source review](../hashicorp-steering.md)

## Summary

Practice Studio helps a doctor prototype a small practice tool with synthetic
data. The doctor can compare bounded treatments, review the source, run fixed
checks, publish a synthetic preview, and export the exact reviewed
application. A developer can then build and change the export without Practice
Studio.

The minimum lovable version uses Docker Compose on a laptop and on one
DigitalOcean Droplet. It does not run Nomad, Vault, or Kubernetes. Gemma is the
only application model. Rust controls source creation, verification, evidence,
and export.

This RFC also describes controls that a later patient data system would need.
Those controls are reference architecture, not claims about the current
runtime.

## Product boundary

The current product is a learning environment for synthetic data. It is not
approved for patient data, clinical care, or production use.

The doctor should reach a useful result and understand what remains. The
product must show a failed check and its reason. The product must not call a
configuration file, simulated response, or planned control observed proof.

## User workflow

```text
choose -> describe -> compare -> review -> check -> repair -> preview -> export
```

The core workflow works without login. Clerk asks for identity only when the
doctor claims or exports a workspace.

The workflow is the stable contract. Infrastructure can change without
changing the doctor's steps or the export evidence format.

## Minimum lovable architecture

The full diagram is in
[decision 0010](../decisions/0010-minimum-lovable-runtime.md).

### Build time

GitHub checks the exact commit. Docker builds the Rust Studio image and the
executable verifier image. CI records immutable image digests. Packer will
create a versioned host image when host replacement time becomes part of the
release gate.

Terraform may create the host, firewall, and DNS. Terraform and Packer are
operator tools. They are not part of the application request path.

### Run time

One DigitalOcean Droplet runs:

- Caddy for TLS and routing;
- Docker Compose;
- the Rust Studio service;
- Postgres;
- one verifier container when source verification is requested.

Netlify serves the exact pull request frontend. The frontend calls the
DigitalOcean staging API. The private DigitalOcean Gemma endpoint receives
only the bounded planning request.

The verifier image is executable and pinned by SHA 256 digest. The verifier
runs fixed checks with networking disabled. The 4 GB host admits one verifier
container at a time.

## Application boundaries

### Gemma planning

Gemma chooses from treatments signed by the selected clinical starter. Gemma
has no tools, file access, secrets, deployment rights, or patient data. Rust
rejects a response that does not match the signed treatment contract.

### Rust source and evidence

Rust:

- creates source from signed rules;
- stores source workspaces and checkpoints;
- binds the accepted digest to the verification report;
- starts the fixed verifier;
- records audit events;
- controls preview and export;
- refuses a patient data release.

### Clinical starters

A starter is a declarative folder under `packs/`. It contains a manifest,
source scaffold, synthetic fixtures, and artifact checks. A starter can narrow
the available treatments and add a fixed check. It cannot grant Gemma new
authority.

### Export

Every export contains:

- the reviewed Svelte client;
- the Rust server;
- tests and synthetic fixtures;
- the accepted checkpoint digest;
- the verification report;
- fixture provenance;
- three editable diagrams;
- one README.

The README explains how to build, change, and run the application with Docker
Compose. It lists the controls that the prototype does not provide. The export
does not depend on DigitalOcean or one scheduler.

## Release evidence

A release claim needs observed proof from the exact source and image digest.
The minimum lovable proof includes:

- a browser run through the complete doctor workflow;
- a shareable Netlify pull request preview;
- an exact commit DigitalOcean staging proof;
- a Gemma provider profile with no fallback;
- a network disabled hosted verifier run;
- a concurrent verifier rejection;
- a restart and persistence check;
- an anonymous export rejection;
- a README only developer handoff;
- profiles for at least 10 exported applications.

The sample profile records build time, bundle size, startup time, memory use,
task completion, customization, and export success.

## Current implementation status

| Area | State | Evidence |
|---|---|---|
| Rust Studio API | observed | `src/api.rs` and contract tests |
| Anonymous synthetic workflow | observed | `scripts/journey.sh` and browser proof |
| Gemma bounded planning | observed on staging | Gemma profile in the evidence index |
| Source checkpoints and exact export | observed | storage and reexport contracts |
| Docker Compose local runtime | observed | `scripts/single-host-smoke.sh` |
| DigitalOcean single host | observed | DigitalOcean proof in the evidence index |
| Executable verifier image | implemented, hosted configuration still required | `verifier/` and verifier contracts |
| One verifier admission limit | implemented, hosted concurrent proof still required | `src/workspace_verifier.rs` |
| Netlify pull request frontend | observed | deploy preview check on pull requests |
| Packer host replacement | planned | no replacement timing proof yet |
| Patient data production controls | planned | reference architecture below |

## When to graduate from one host

Add a separate sandbox worker when any of these conditions is true:

- hosted generated code must run outside fixed verification;
- more than one practice needs a proved isolation boundary;
- concurrent verification cannot fit safely on the 4 GB host.

Add a separate production workload boundary before any workload can receive
patient data. Add tenant data boundaries, short lived workload credentials,
owned encryption keys, durable audit storage, backups, restore tests,
monitoring, and an incident process before making a production claim.

## Optional reference architecture

A later team may choose Nomad for workload scheduling and Vault for workload
identity, short lived credentials, and encryption keys. Another team may use
different services. The doctor's workflow, export shape, and evidence format
must not depend on that choice.

The old files under `nomad/`, `vault/`, `terraform/prod/`, and the old Packer
templates record earlier architecture work. They are not setup instructions
for the minimum lovable version.

The first reference step separates the control plane and sandbox worker. The
sandbox worker has no route to a tenant database. The next step adds a separate
production workload boundary and managed data services.

Kubernetes is not a supported deployment path. The project will not maintain
two schedulers.

## Design patterns taken from HashiCorp

The team read source and commit history from Nomad, Vault, Packer, Waypoint,
and Boundary. We use their design patterns without requiring their runtimes.

- Small interfaces have explicit unsupported capabilities.
- Validation happens before side effects.
- Desired state and observed state remain separate.
- Each operation is recorded before work starts so a crash is visible.
- An operation that needs durable audit evidence fails when no durable sink
  accepts the event.
- Operator access uses an explicit target, policy, session limit, and
  termination reason.

The detailed source notes are in
[HashiCorp design steering](../hashicorp-steering.md).

## Alternatives

### Add Nomad and Vault to the minimum lovable runtime

This would add deployment and secret service work before the doctor workflow
and export handoff are proved. The current product does not need these services
to handle synthetic data. We keep them as optional reference choices.

### Add Kubernetes

This would create a second scheduler path. The team would have to test and
document two systems without improving the current doctor workflow. We reject
this option.

### Use only a managed application platform

A managed platform is a reasonable frontend or export target. The current
DigitalOcean Droplet remains the standard because it proves the same Docker
Compose shape that a person can run locally and gives the verifier fixed host
limits.

## Open work

Issue #68 owns this product and architecture decision. The linked issues own
the remaining implementation:

- #11 owns the README only developer handoff;
- #28 owns editable source workspaces and executable checks;
- #3 owns runtime release evidence;
- #6 owns later isolated workload work;
- #8 owns durable audit storage;
- #9 owns short lived workload identity;
- #10 owns clinician and operator identity;
- #30 owns signed starter contents.
