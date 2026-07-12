# DigitalOcean production proof

Exact source: `9eabeedcd58f673cb6f6aa071cb75c735d17fce3`.

Environment: DigitalOcean Droplet `584090793`, `nyc3`, 2 vCPU / 4 GB,
Ubuntu, Docker Compose, Postgres 16, restricted firewall. The studio container
runs as uid 65532 with a read-only root filesystem, all Linux capabilities
dropped, and persistent state on named volumes.

`scripts/single-host-release.sh` fetched the exact commit, rebuilt the pinned
multi-stage image, recreated the studio, waited for health, and ran the remote
portability proof. Result: passed after every GitHub check for the same SHA
completed successfully. The optimized Rust build took 1m14s.

The remote proof created `remote-portability-proof-1783887786`, kept it on
synthetic data, exported its owned Rust/documentation/deploy bundle, and
confirmed that the operate response makes no fabricated telemetry claim.

Thirty authenticated `/api/packs` requests from the local operator measured
32.2 ms p50, 42.1 ms p95, and 47.0 ms maximum. Browser inspection found 31
visible interactive controls at mobile width; minimum computed height was
44 px and none were below the target.

The browser began unauthenticated and observed the two expected 401 responses,
entered the strict-mode token into tab-scoped session storage through the
visible sign-in form, and then rendered all 17 starters plus the newly created
remote proof app. No token was placed in the URL or persisted server-side.

Screenshots:

- [desktop](screenshots/digitalocean/production-desktop.png)
- [mobile](screenshots/digitalocean/production-mobile.png)

The public endpoint is HTTP and IP-restricted for synthetic proof. TLS, OIDC,
rotated production credentials, and separate production/staging hosts remain
required before patient data is permitted.
