# DigitalOcean production proof

Exact source: `49b69d96bf168998ff6b2dcc51892bb2f97ddb8f`.

Environment: DigitalOcean Droplet `584090793`, `nyc3`, 2 vCPU / 4 GB,
Ubuntu, Docker Compose, Postgres 16, restricted firewall. The studio container
runs as uid 65532 with a read-only root filesystem, all Linux capabilities
dropped, and persistent state on named volumes.

`scripts/single-host-release.sh` fetched the exact commit, rebuilt the pinned
multi-stage image, recreated the studio, waited for health, and ran the remote
portability proof. Result: passed. Changed-code release time was 1m42.59s;
the optimized Rust build was 1m16s.

The remote proof created `remote-portability-proof-1783883918`, kept it on
synthetic data, exported its owned Rust/documentation/deploy bundle, and
confirmed that the operate response makes no fabricated telemetry claim.

Thirty authenticated `/api/packs` requests from the local operator measured
34.7 ms p50, 47.8 ms p95, and 78.7 ms maximum. Browser inspection found 30
visible interactive controls at mobile width; minimum computed height was
44 px and none were below the target.

Screenshots:

- [desktop](screenshots/digitalocean/production-desktop.png)
- [mobile](screenshots/digitalocean/production-mobile.png)

The public endpoint is HTTP and IP-restricted for synthetic proof. TLS, OIDC,
rotated production credentials, and separate production/staging hosts remain
required before patient data is permitted.
