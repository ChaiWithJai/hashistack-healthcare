# Practice Studio

[![Rust checks](https://github.com/ChaiWithJai/hashistack-healthcare/actions/workflows/ci.yml/badge.svg)](https://github.com/ChaiWithJai/hashistack-healthcare/actions/workflows/ci.yml)
[![User journey](https://github.com/ChaiWithJai/hashistack-healthcare/actions/workflows/evals.yml/badge.svg)](https://github.com/ChaiWithJai/hashistack-healthcare/actions/workflows/evals.yml)
[![Netlify Status](https://api.netlify.com/api/v1/badges/8956042c-9a19-49f0-9a4c-31f808cff96e/deploy-status)](https://app.netlify.com/projects/gethoursback/deploys)

Practice Studio helps clinicians build small practice tools with synthetic data.
A clinician can describe a tool, compare three proposed treatments, review the
source change, run checks, publish a synthetic preview, and export the source.

You do not need an account to build or test a synthetic tool. Clerk asks you to
sign in only when you claim a workspace or export its source and audit record.

> Practice Studio is a learning environment. It is not approved for patient
> data, clinical care, or production use.

## Project links

* [Netlify production candidate](https://gethoursback.netlify.app)
* [DigitalOcean staging](https://138-197-27-225.sslip.io)
* [Documentation](docs/README.md)
* [Current evidence and limits](docs/evidence-index.md)
* [Issue board](https://github.com/ChaiWithJai/hashistack-healthcare/issues)

The Netlify site and DigitalOcean staging service currently share the same Rust
backend. Keep all data synthetic. Read [Deploy Practice Studio](docs/deployment.md)
before you create another hosted environment.

## Run the local demo

Install these tools:

* Git
* Docker with Compose version 2
* `curl`
* Python 3

Give Docker at least 4 GB of memory. Then clone the repository and run the
supported proof.

```bash
git clone git@github.com:ChaiWithJai/hashistack-healthcare.git
cd hashistack-healthcare
scripts/single-host-smoke.sh
```

The script builds the Rust service, starts Postgres, waits for both services,
and tests the synthetic release path. A successful run ends with this message.

```text
single-host proof passed: <app-id> survived restart; real promotion denied; synthetic export succeeded
```

Open [http://localhost:3000](http://localhost:3000).

## Complete the demo

1. Choose a signed clinical starter.
2. Describe a small practice tool.
3. Compare the three proposed treatments.
4. Select one treatment and review the source change.
5. Review the five checks and accept the candidate.
6. Open the release check and repair the named failure.
7. Publish the synthetic preview.
8. Select "Make this mine" when you are ready to sign in and export.

Stop the services and keep the local database.

```bash
docker compose down
```

Delete the services and all local Practice Studio data.

```bash
docker compose down --volumes
```

The second command deletes data. Read
[Run Practice Studio locally](docs/get-started/local.md) for setup errors and
restart steps.

## What the demo runs

The browser calls one Rust control plane. The control plane stores workspace
state. It handles each model request and release check. It also stores
deployment records and the audit log.

```text
browser
  -> Rust API
     -> workspace planner and source generator
     -> signed clinical packs
     -> one network-disabled verifier container
     -> Postgres and audit log
     -> synthetic preview and owned export
```

The default local setup uses deterministic planning and source generation. A
hosted setup can use the private DigitalOcean Gemma planner. The Rust service
checks every hosted response before it changes a workspace.

Read the [minimum lovable runtime decision](docs/decisions/0010-minimum-lovable-runtime.md)
for the current design. The [platform RFC](docs/rfc/0001-clinician-platform.md)
explains the later reference architecture.

## Run the checks

Run the checks used for normal code review.

```bash
make check
scripts/docs-check.sh
```

Run the browser journey and exported application proof.

```bash
scripts/journey.sh
```

Nomad and Vault are not part of the supported local runtime. The repository
keeps an older integration proof for architecture research, but you do not
need it to build, test, preview, or export an application.

See the [command reference](docs/reference/commands.md) for each command and the
state it changes.

## Extend the project

Use the existing boundaries instead of adding logic to the web page.

* Add a clinical starter in `packs/<name>/`.
* Change the Gemma planner adapter behind `src/workspace_agent.rs` without adding another application model.
* Add a release rule in `src/gates.rs` and include evidence for the rule.
* Add a deployment provider behind `src/deploy.rs`.
* Add API behavior in `src/api.rs` and prove it through a contract test.

Every exported application must include these files:

* A Svelte client.
* A Rust server.
* Synthetic fixtures.
* Exactly three editable diagrams.
* One README and no other documentation files.

Read [HashiCorp design steering](docs/hashicorp-steering.md) before you add a
pack, provider, release rule, or deployment path.

## Repository map

| Path | Purpose |
|---|---|
| `src/` | Rust control plane |
| `web/` | Static Practice Studio client |
| `packs/` | Signed clinical starters and synthetic fixtures |
| `tests/` | API, identity, evidence, storage, and release contracts |
| `verifier/` | Fixed Svelte, Rust, and browser checks for source candidates |
| `scripts/` | Local proofs, hosted proofs, and evaluation commands |
| `terraform/` | DigitalOcean, GCP, Cloudflare, and single host setup |
| `docs/` | Tutorials, operations guides, reference, decisions, and evidence |

## Deploy previews

Netlify builds each pull request from its exact frontend commit. The pull
request contains a `Deploy Preview` check with a shareable URL. Numbered preview
hosts can create anonymous synthetic workspaces through the DigitalOcean
staging API. Preview hosts cannot claim or export a workspace with Clerk.

The Rust commit still needs a separate staging deployment and proof. Read
[Try the hosted preview](docs/get-started/hosted-preview.md) for the review
steps.

## Contribute

Keep each change tied to one user problem. Name the exact commit and the command
that proves the change. State any limit that remains. Follow
[CONTRIBUTING.md](CONTRIBUTING.md) and the
[merge standard](docs/process/merge-standard.md).

## License

This project uses the license in [LICENSE](LICENSE).
