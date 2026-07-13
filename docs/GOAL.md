# The goal and the bar

## The goal

Practice Studio helps a doctor turn a small practice problem into an owned
prototype. The doctor uses synthetic data and does not need an account while
building. Clerk asks for identity only when the doctor claims or exports the
workspace.

Two moments must work.

### Build something useful

The doctor chooses a signed clinical starter and describes the job in plain
language. Gemma proposes bounded treatments. The doctor compares them, accepts
one, inspects the source change, and runs fixed checks. The doctor can repair a
named failure and publish a synthetic preview.

Gemma is the only application model. It cannot use tools, read files, access
secrets, deploy code, or receive patient data. Rust checks every proposal and
owns source creation and release decisions.

### Own the result

The doctor can export the exact source that was reviewed. The export contains:

- a Svelte client;
- a Rust server;
- tests and synthetic fixtures;
- the accepted checkpoint digest;
- the verification report;
- three editable diagrams;
- one README with local Docker Compose instructions.

A developer should be able to use only that README to build, change, and run
the application. The README must explain which controls are missing before the
prototype can use patient data.

The export is the product handoff. It must not depend on Practice Studio,
DigitalOcean, Nomad, Vault, or another infrastructure vendor after download.

## The minimum lovable bar

The minimum lovable version is complete when all of these results are
observed:

| Result | Proof |
|---|---|
| The core synthetic workflow works without login | Browser journey starts in a clean browser and reaches preview |
| Identity appears only at claim or export | Browser journey proves Clerk is absent from the build flow and required at export |
| A pull request has a shareable preview | Netlify reports the exact pull request frontend commit |
| The preview uses DigitalOcean staging | The preview calls the staging API and reports its exact Rust commit |
| Gemma stays bounded | Provider profile and adversarial contract tests prove signed treatment selection and invalid response rejection |
| Rust creates and checks source | Checkpoint and verifier contracts bind the accepted source digest to the report |
| Verification is contained | The hosted verifier has no network and admits one container at a time on the 4 GB host |
| Failures help the doctor continue | Browser proof shows the failed check, its reason, and the repair path |
| The export belongs to the doctor | Reimport and reexport preserve the reviewed source map and digest |
| A stranger can continue the work | A person completes the README only build, change, and run proof |
| The result works across the sample | At least 10 exports record build time, bundle size, startup time, memory, task completion, customization, and export success |

## Supported runtime

Docker Compose is the application runtime on a laptop and on one DigitalOcean
Droplet. The host runs Caddy, the Rust Studio service, Postgres, and at most one
verifier container. Netlify serves pull request frontends. Terraform may create
the host, firewall, and DNS. Packer will create a versioned host image when the
team adds host replacement time to the release gate.

Nomad, Vault, and Kubernetes are not part of the minimum lovable runtime. The
repository keeps older Nomad and Vault work as research and as a possible
reference for a later system. See
[decision 0010](decisions/0010-minimum-lovable-runtime.md).

## Product boundary

Practice Studio is a learning environment for synthetic data. It is not
approved for patient data, clinical care, or production use.

The minimum lovable version does not claim to provide:

- tenant isolation for patient data;
- verified clinician identity;
- short lived workload credentials;
- owned encryption keys and rotation;
- durable off host audit retention;
- production backups and tested restore;
- a patient facing workload boundary;
- high availability.

We add a separate sandbox worker before generated code can run outside fixed
verification. We add a separate production workload boundary and the controls
above before any patient data can enter the system.

## Non goals

- We do not support two schedulers.
- We do not add another application model.
- We do not turn the minimum lovable version into a broad infrastructure
  platform.
- We refuse use cases that need enterprise outcomes analysis, regulated
  interoperability, clinical triage, or medical device behavior.
