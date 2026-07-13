# Command reference

Run commands from the repository root unless the entry says otherwise.

## Product lifecycle

| Command | Purpose | Changes state |
|---|---|---|
| `scripts/single-host-smoke.sh` | Start Compose and prove the synthetic studio flow | yes |
| `scripts/journey.sh` | Drive the clinician flow and exported app in a browser | writes ignored proof |
| `scripts/staging-docker-up.sh` | Reproduce the historical Nomad and Vault integration proof | yes |
| `scripts/staging-docker-up.sh down` | Stop the historical integration proof | yes |
| `scripts/pressure-test.sh <url>` | Test the control plane and configured infrastructure | yes |
| `scripts/single-host-remote-proof.sh <url>` | Prove a running remote synthetic deployment | yes |

## Code checks

| Command | Purpose |
|---|---|
| `cargo fmt --check` | Check Rust formatting |
| `cargo clippy --all-targets -- -D warnings` | Reject Rust warnings |
| `cargo test` | Run Rust unit and integration tests |
| `scripts/evals.sh` | Score the pack and user scenarios |
| `scripts/docs-check.sh` | Check local documentation links and command references |

## Containers

| Command | Purpose |
|---|---|
| `docker compose ps` | Show service and health state |
| `docker compose logs studio` | Show service logs |
| `docker compose down` | Stop services and retain volumes |
| `docker compose down --volumes` | Stop services and delete local data |

## Terraform

Use `terraform -chdir=<directory>` from the repository root.

| Directory | Purpose |
|---|---|
| `terraform/single-host/digitalocean` | Disposable synthetic DigitalOcean host |
| `terraform/single-host/gcp` | Equivalent synthetic GCP host |
| `terraform/cloudflare` | Owned staging, preview, and production DNS |
| `terraform/prod` | Future multi-node production substrate, not the current release path |

Always run `terraform plan` before `terraform apply`. Never commit a
`.tfvars` file or Terraform state.
