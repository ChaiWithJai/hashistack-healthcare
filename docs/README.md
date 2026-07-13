# Practice Studio documentation

Practice Studio helps a clinician describe a small tool, test it with synthetic
data, check its release requirements, and take the source away.

Start with one path. Use the reference only when you need a detail.

## Get started

- [Run Practice Studio locally](get-started/local.md)
- [Run the complete stack locally](../terraform/single-host/README.md)
- [Try the hosted staging site](get-started/hosted-preview.md)
- [Understand the product boundary](product-use-case.md)

## Operate

- [Run staging and production](deployment.md)
- [Troubleshoot setup and previews](troubleshooting.md)
- [Configure Clerk](operations/clerk.md)
- [Respond to an operational problem](ops-runbook.md)
- [Provision the DigitalOcean host](digitalocean-runbook.md)

## Extend

- [Read the platform RFC](rfc/0001-clinician-platform.md)
- [Add packs, gates, and agent providers](hashicorp-steering.md)
- [Understand access control](reference/access-control.md)
- [Use the command reference](reference/commands.md)
- [Review architecture decisions](decisions/)
- [Understand the hosted builder and model routing](decisions/0009-agent-workspace-and-model-routing.md)

## Verify

- [See what is proved and what is not](evidence-index.md)
- [Run the clinician journey](../scripts/journey.sh)
- [Review the app sample](evals/sample-artifact-profiles.md)
- [Follow the merge standard](process/merge-standard.md)
- [Apply the documentation standard](documentation-standard.md)

## Historical material

Design studies, investigations, and dated proof reports explain how decisions
were reached. They are evidence, not setup instructions. Start with the pages
above before reading those records.
