# Documentation standard

Practice Studio uses Vagrant's reader split as its benchmark.

Benchmark sources:

- [Vagrant get-started tutorials](https://developer.hashicorp.com/vagrant/tutorials/get-started)
- [Vagrant documentation index](https://developer.hashicorp.com/vagrant/docs)
- [Vagrant command reference](https://developer.hashicorp.com/vagrant/docs/cli)
- [Vagrant provider configuration](https://developer.hashicorp.com/vagrant/docs/providers/configuration)

The project last checked these sources on July 12, 2026.

## Benchmark

| Reader need | Vagrant pattern | Practice Studio page | Check |
|---|---|---|---|
| Understand the product | short introduction | repository README | first screen names user, outcome, and safety limit |
| Complete the first lifecycle | ordered get-started tutorial | `get-started/local.md` | prerequisites, commands, expected state, cleanup |
| Find a command | CLI reference | `reference/commands.md` | command, purpose, state change |
| Configure a provider | provider reference | Terraform module README and deployment guide | inputs and safe defaults |
| Operate and recover | networking and provisioning guides | deployment and troubleshooting pages | health, logs, restart, rollback |
| Extend the system | provider and plugin docs | RFC and HashiCorp steering | extension boundary and contracts |

## Rules

- The README is a front door, not a proof archive.
- Tutorials use one supported path and state what success looks like.
- Reference pages describe facts. They do not tell a story.
- Operations pages include failure and cleanup behavior.
- Dated measurements stay in `docs/evals` or `docs/evidence`.
- A command shown in current documentation must exist.
- A local Markdown link must resolve.
- Generated screenshots and bundles stay outside Git.

Run `scripts/docs-check.sh` before review.
