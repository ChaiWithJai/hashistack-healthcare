# Deploy Practice Studio

The supported hosted shape is Cloudflare in front of DigitalOcean. See
[decision 0008](decisions/0008-cloudflare-delivery-boundary.md) for the tradeoffs.

## Environments

| Environment | Workload | Login | Data |
|---|---|---|---|
| Pull request | isolated container on staging host | none | synthetic only |
| Staging | fixed service on staging host | Clerk development instance | synthetic only |
| Production | separate host and tunnel | Clerk production instance | production remains blocked until its release gate is met |

## DNS and tunnels

Before deployment, choose a domain in a Cloudflare-managed zone and create two
tunnels. The staging tunnel serves the staging name and wildcard preview name.
The production tunnel serves only the production name.

Copy `terraform/cloudflare/terraform.tfvars.example`, fill in non-secret IDs
and names, then provide `CLOUDFLARE_API_TOKEN` through the environment:

```bash
terraform -chdir=terraform/cloudflare init
terraform -chdir=terraform/cloudflare plan
```

The token needs DNS edit access for one zone. Tunnel credentials do not belong
in Terraform state. Install each tunnel token directly on its host.

## GitHub environments

Create protected `infrastructure`, `staging`, and `production`
environments.

Infrastructure uses:

- secret `CLOUDFLARE_API_TOKEN`;
- variable `CLOUDFLARE_ZONE_ID`;
- variable `CLOUDFLARE_ZONE_NAME`;
- variable `STAGING_TUNNEL_ID`;
- variable `PRODUCTION_TUNNEL_ID`.
- secrets `TERRAFORM_STATE_ACCESS_KEY` and
  `TERRAFORM_STATE_SECRET_KEY` for one DigitalOcean Spaces bucket;
- variables `TERRAFORM_STATE_BUCKET` and `TERRAFORM_STATE_REGION`.

The DNS workflow keeps Terraform state in that Spaces bucket. Do not run an
apply from an Actions runner with local state.

Staging and production each use their own SSH host, key, known-host entry,
Clerk values, Postgres password, audit HMAC key, and anonymous-session HMAC
key. Do not copy staging secrets into production.

## Release rule

CI builds once and records the commit and image digest. A preview or staging
deploy must report both values. Production may accept only a digest that
already passed staging. Rollback selects an earlier proved digest.

Until digest-based deployment replaces the current source-over-SSH workflow,
the existing DigitalOcean job remains a staging portability proof. It is not a
production promotion path.

## Clerk cutover

The production Clerk instance needs an owned application domain. Complete its
DNS checks only after `app.<domain>` resolves through Cloudflare. Preview
hosts are not Clerk authorized parties and cannot claim or export a workspace.

## Verify

For each environment:

```bash
curl -fsS https://<host>/health
scripts/single-host-remote-proof.sh https://<host>
```

The proof must create an anonymous workspace, change a synthetic app, reject a
real release, publish a synthetic preview, and reject anonymous export.
