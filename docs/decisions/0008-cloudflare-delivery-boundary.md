# Decision 0008: Cloudflare owns delivery, DigitalOcean runs the service

Status: accepted

## Context

The current staging URL uses `sslip.io`. We do not control that DNS zone.
That prevents a complete Clerk production setup and makes stable pull request
URLs difficult.

Practice Studio is a stateful Rust service. Its browser UI and API share one
origin. A static site host would add a proxy and a second deployment system
without removing the need for the Rust origin.

## Decision

DigitalOcean runs the containers. Cloudflare owns public DNS, edge TLS, and the
private path to each origin through Cloudflare Tunnel.

Use two hosts:

- staging and synthetic pull request previews share one DigitalOcean Droplet;
- production uses a separate Droplet, VPC, tunnel, database, and secret set.

Use three DNS records in one owned zone:

- `staging.<domain>` points to the staging tunnel;
- `*.preview.<domain>` points to the staging tunnel;
- `app.<domain>` points to the production tunnel.

One wildcard preview record is enough. CI must not create and delete DNS for
each pull request.

Each build produces one immutable image digest. Staging proves that digest.
Production promotion deploys the same digest after human approval. It does not
rebuild source.

Pull request previews are anonymous and synthetic. Clerk is configured only on
the fixed staging and production origins. Export remains unavailable in an
anonymous preview.

## Why not Cloudflare Pages

Pages gives excellent static preview URLs. It does not remove the dynamic Rust
service, database, audit store, or same-origin login boundary. A Pages proxy
would create another runtime and another place to debug cookies and caching.
We can reconsider Pages if the web client becomes a separate static build.

## Security boundary

- `cloudflared` opens outbound connections. Application ports are not public.
- Preview containers have no Clerk, production, Vault, or tenant secrets.
- Cloudflare bypasses cache for `/api/*`, login callbacks, and health checks.
- Production has no shared volumes, networks, or host with previews.
- The Cloudflare token is limited to the owned zone and lives only in the
  protected GitHub `infrastructure` environment.

## Consequences

Cloudflare becomes a delivery dependency. The service remains testable over
loopback and SSH if the tunnel is unavailable. The extra production Droplet is
the minimum cost required to keep review code away from production.
