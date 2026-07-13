# Minimum lovable hosted inventory on 2026-07-13

## Scope

This record covers the DigitalOcean resources used by Practice Studio. The
commands were run from an authenticated local `doctl` session. No resource was
changed.

## Observed resources

| Resource | Observed configuration | Cost basis |
|---|---|---|
| `hashistack-healthcare-studio` Droplet | `nyc3`, `s-2vcpu-4gb`, 4 GB memory, weekly backups enabled | $24 per month for the Droplet plus $4.80 for the basic weekly backup plan |
| `practice-studio-treatment-planner` agent | `tor1`, Gemma 4 | Agent creation is free. Gemma 4 usage is $0.18 per million input tokens and $0.50 per million output tokens. |

The fixed DigitalOcean host cost is $28.80 per month before transfer or other
usage charges. Gemma cost depends on observed token use and has no fixed
monthly charge. The current inventory command does not return billed token
use, so this record does not invent a monthly Gemma total.

DigitalOcean lists the Droplet size at $24 per month. DigitalOcean's
[backup pricing](https://docs.digitalocean.com/products/backups/details/pricing/)
adds 20 percent for the basic weekly plan. DigitalOcean's
[inference pricing](https://docs.digitalocean.com/products/inference/details/pricing/)
lists the Gemma 4 token rates.

## Commands used

```sh
doctl compute droplet list
doctl compute droplet get hashistack-healthcare-studio --output json
doctl compute size list --output json
doctl genai agent list
doctl genai list-models --output json
```

## Teardown

Terraform owns the Droplet, firewall, and related project resources. Destroy
them together:

```sh
terraform -chdir=terraform/single-host/digitalocean destroy
```

The Gemma agent is a separate resource. Find its identifier by name and review
it before deletion:

```sh
doctl genai agent list
doctl genai agent delete <agent-id>
```

The second command asks for confirmation. Do not use `--force` in the runbook.

Export owned applications, the audit stream, and a database backup before
destroying a resource that people still use.

## Limits

This inventory proves which resources existed and their public price basis. It
does not prove a billing total, restore process, or production readiness.
Netlify is outside DigitalOcean and is not included in the $28.80 figure.
