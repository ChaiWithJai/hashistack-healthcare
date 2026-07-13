# Practice Studio

Practice Studio helps a clinician describe a small practice tool, change it
with synthetic data, check it before release, and export the source.

You do not need an account to explore the core flow. Clerk is required when you
claim a workspace and export it.

> Practice Studio is not approved for patient data or clinical care. Local,
> pull request, and staging environments use synthetic data only.

## Try it

Run the supported local proof:

```bash
scripts/single-host-smoke.sh
```

Open [http://localhost:3000](http://localhost:3000). Then:

1. Choose a clinical starter.
2. Describe the tool.
3. Ask for one change.
4. Run the release check.
5. Repair the named failure.
6. Publish a synthetic preview.

Stop the stack without deleting its database:

```bash
docker compose down
```

For prerequisites, expected output, reset behavior, and common failures, read
[Run Practice Studio locally](docs/get-started/local.md).

## Hosted staging

The current synthetic staging site is
[https://138-197-27-225.sslip.io](https://138-197-27-225.sslip.io).

The temporary `sslip.io` address is not the production domain. The supported
delivery design uses an owned Cloudflare zone, Cloudflare Tunnel, and separate
DigitalOcean hosts for staging and production. Read
[Deploy Practice Studio](docs/deployment.md).

## What the repository proves

CI checks:

- Rust formatting, linting, and tests;
- every pack scaffold;
- the local Nomad, Vault, and Postgres pressure path;
- the complete clinician journey;
- the exported application build and browser flow;
- DigitalOcean, GCP, and Cloudflare Terraform contracts.

The current evidence and limits are in
[What is proved](docs/evidence-index.md). Dated counts belong in generated
evaluation reports, not in this README.

## Documentation

Use the [documentation map](docs/README.md) to find:

- local and hosted setup;
- operations and login;
- extension points;
- API and command reference;
- proof reports and architecture decisions.

The documentation follows the same reader split used by Vagrant. A short
tutorial teaches the first lifecycle. Reference pages describe commands and
configuration. Operations pages cover deployment and recovery.

## Architecture

The request path is:

```text
browser -> Rust API -> pack, agent, gate, deploy, audit
```

The core extension points are packs, agent providers, and release gates. The
service keeps generation, checks, release records, and audit events separate so
one component can change without changing the clinician flow.

Read:

- [Platform RFC](docs/rfc/0001-clinician-platform.md)
- [HashiCorp design steering](docs/hashicorp-steering.md)
- [Cloudflare delivery decision](docs/decisions/0008-cloudflare-delivery-boundary.md)
- [Access control reference](docs/reference/access-control.md)

## Develop

```bash
cp env.example .env
cargo run
```

The service runs at [http://127.0.0.1:3000](http://127.0.0.1:3000).

Run the local checks:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
scripts/docs-check.sh
```

Run the infrastructure proof:

```bash
scripts/staging-docker-up.sh
# Follow the printed command in a second terminal.
scripts/staging-docker-up.sh down
```

Run the complete user and exported-app journey:

```bash
scripts/journey.sh
```

Generated screenshots, bundles, browser traces, Terraform values, and local
state are ignored by Git.

## Contribute

Use the issue board for scoped work. Each open issue has an area, priority,
status, milestone, current state, and testable acceptance criteria.

Follow the [merge standard](docs/process/merge-standard.md). A pull request must
identify its exact commit, proof, limitations, and hosted preview status.
