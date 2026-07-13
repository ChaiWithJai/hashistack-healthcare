# Frontend delivery infrastructure: what's live vs. what's staged

This is a read-only tour of how a browser request actually reaches Practice
Studio today, and what infrastructure exists in the repo but is not yet wired
into that path. Every claim below quotes the file that proves it.

## 1. What's live today: Netlify → hardcoded DigitalOcean IP

`netlify.toml` is the only thing serving the frontend. It publishes the
static `web/` build and does nothing else except proxy three path prefixes to
one hardcoded droplet IP:

```toml
[build]
  publish = "web"

[[redirects]]
  from = "/api/*"
  to = "https://138-197-27-225.sslip.io/api/:splat"
  status = 200
  force = true

[[redirects]]
  from = "/auth/*"
  to = "https://138-197-27-225.sslip.io/auth/:splat"
  status = 200
  force = true

[[redirects]]
  from = "/health"
  to = "https://138-197-27-225.sslip.io/health"
  status = 200
  force = true
```

The comment above those redirects in the file states the current shape
directly: "The browser stays on the Netlify origin while the Rust control
plane remains on the DigitalOcean droplet. Replace this target when
production receives a dedicated backend." That target — `138-197-27-225.sslip.io`
— is a raw DigitalOcean droplet IP resolved through the free `sslip.io`
wildcard-DNS service, not a domain anyone owns. `netlify.toml` also declares
`[context.deploy-preview]` and `[context.branch-deploy]`, both publishing
`web` — this is Netlify's own built-in PR/branch preview mechanism, separate
from and in addition to the DigitalOcean staging-preview flow described
below. Every Netlify preview, whether a PR deploy-preview or a named-branch
deploy, proxies `/api/*`, `/auth/*`, and `/health` to that same single
hardcoded IP — there is no per-preview backend.

So the live pipeline is:

```
Browser
  -> Netlify (static web/ build, deploy-preview / branch-deploy / prod contexts)
     -> [asset request]         served directly from Netlify's CDN
     -> [/api/*, /auth/*, /health]  redirected (force=true, 200) to
                                    https://138-197-27-225.sslip.io/...
                                    (one shared DigitalOcean droplet)
```

There is exactly one backend origin for every Netlify context. Nothing in
`netlify.toml` distinguishes preview traffic from production traffic at the
proxy layer.

## 2. The DigitalOcean side: one shared droplet, provisioned and proven

`docs/digitalocean-runbook.md` documents a "Proven baseline" dated
2026-07-12: the `terraform/single-host/digitalocean` module provisioned one
Ubuntu 24.04 droplet in `nyc3` on a `s-2vcpu-4gb` size (2 vCPU, 4 GiB, 80 GiB,
"$24/month before backups"), with backups, IPv6, DigitalOcean monitoring, and
a cloud firewall restricted to the operator's `/32` for SSH and port 3000.
Measured results quoted directly from the runbook table: cloud-init reported
`done` with no errors, first boot to ready took "about 7 minutes," the Rust
release build within first boot took "3m38s," runtime hardening confirmed
"UID/GID 65532, read-only root filesystem, all capabilities dropped," and
health-check latency was "p50 33.1 ms, p95 39.5 ms over 30 requests."

The runbook is explicit that this is a single shared host, not
high-availability: "A shared Droplet is an explicit hobby-MVP cost
trade-off, not high availability. Run two Compose projects (`staging` and
`prod`) ... Never mount the Docker socket into either application." The
promotion model is digest-based: "The promotion path is an immutable image
digest: staging proves the digest, then production adopts that exact
digest. Do not rebuild between environments."

### The staging-preview GitHub Actions flow, end to end

`.github/workflows/staging-preview.yml` is a `workflow_dispatch` job
(manually triggered with a PR number as input, not automatic on every push).
Its steps, in the order they run:

1. **Resolve the exact PR head SHA.** It calls `gh pr view "$PR" --json state`
   to confirm the PR is `OPEN`, then `gh pr view "$PR" --json headRefOid` to
   get the head commit SHA, asserting it is exactly 40 characters.
2. **Checkout that exact SHA**, then re-verify with
   `test "$(git rev-parse HEAD)" = "$COMMIT_SHA"` — a belt-and-suspenders
   check that the checked-out tree really is the PR head, not a merge ref.
3. **Configure SSH** to the staging droplet using secrets
   `DO_STAGING_SSH_KEY`, `DO_STAGING_KNOWN_HOSTS`, and `DO_STAGING_HOST`,
   pinning the known-hosts entry (`ssh-keygen -F "$STAGING_SSH_HOST" ...`).
4. **Open a temporary firewall hole for the runner's IP.** The step runs
   `runner_cidr=$(scripts/digitalocean-runner-firewall.sh open)` and installs
   `trap cleanup EXIT` where `cleanup()` calls
   `scripts/digitalocean-runner-firewall.sh close "$runner_cidr"` — so the
   hole is closed whether the deploy succeeds or fails. Reading
   `scripts/digitalocean-runner-firewall.sh` directly: `open` resolves the
   runner's public IPv4 via `https://api.ipify.org`, turns it into a `/32`,
   and `POST`s a firewall rule `{protocol:"tcp",ports:"22",sources:{addresses:[$cidr]}}`
   to the DigitalOcean firewalls API; `close` `DELETE`s that exact same rule.
   Only port 22 for that one IP is ever opened — application ports 80/443
   stay public per the runbook.
5. **Deploy the exact PR SHA and prove the critical path**: after an SSH
   reachability check (`ssh ... true`), it runs
   `scripts/single-host-configure-agent.sh "$STAGING_SSH_HOST"` (wires up the
   DigitalOcean planner endpoint/key/version into the host env file) then
   `scripts/single-host-release.sh "$STAGING_SSH_HOST" "$COMMIT_SHA"` to
   actually deploy that commit.
6. **Comment the (fixed) staging URL back on the PR**:
   `gh pr comment "$PR" --body "DigitalOcean staging preview: $STAGING_URL. Deployed exact SHA \`$COMMIT_SHA\`. ..."`
   — `STAGING_URL` comes from the repo/environment variable `DO_STAGING_URL`,
   i.e. it is one fixed URL for every PR, not a synthesized per-PR subdomain.

The job has `concurrency: group: digitalocean-staging, cancel-in-progress: false`
— only one staging deploy can run at a time, and a new dispatch waits rather
than cancelling an in-flight one, since there is only one shared droplet.

### The DigitalOcean Gemma planner (native Rust, not a Python microservice)

Per `docs/decisions/0009-agent-workspace-and-model-routing.md`, planning
logic lives natively in the Rust control plane
(`src/workspace_agent.rs`, `src/workspace.rs`), which calls a private
DigitalOcean-hosted Gemma 4 endpoint. The ADR states the boundary precisely:
"Gemma cannot write files, run commands, deploy, read production data, use
GitHub, or receive platform secrets," and "Rust checks the ID and rebuilds
the full treatment from the signed pack rules before the user can choose
it" — the model's response is treated as untrusted data, never executed
directly. The ADR also explicitly rules out an alternative architecture: "We
reviewed Open SWE ... and Deep Agents as prior art. They are comparison
points, not dependencies or deployed services ... This avoids a Python
worker, another state store, and another model." Git history confirms a
`services/agent/` Python microservice existed and was deleted:
`git log --diff-filter=D -- services/agent` shows it removed in commit
`0bccc87 feat: activate versioned DigitalOcean planning (#51)`, consistent
with the ADR's decision to keep this native to Rust rather than run a
separate Python service.

`docs/digitalocean-runbook.md` names the concrete deployed agent: "The
staging planner is a private DigitalOcean agent named
`practice-studio-treatment-planner` in `tor1`. It uses Gemma 4." The release
workflow copies "its endpoint, scoped key, and immutable version" into the
host's env file, and "the remote proof requires the returned workspace to
report that exact provider, model, and version with no fallback." The
runbook is blunt about data handling: "The DigitalOcean agent must receive
synthetic data only. Do not send patient data to the agent until
DigitalOcean confirms the required health care terms and the full data
retention path in writing."

## 3. What's staged but NOT cut over: Cloudflare DNS/tunnel routing

`docs/decisions/0008-cloudflare-delivery-boundary.md` is an **accepted**
ADR ("Status: accepted") that decides Cloudflare should own public DNS, edge
TLS, and tunnel routing to the DigitalOcean droplets, replacing the current
`sslip.io` arrangement. Its stated motivation, quoted directly: "The current
staging URL uses `sslip.io`. We do not control that DNS zone. That prevents
a complete Clerk production setup and makes stable pull request URLs
difficult." It also explicitly rejects Cloudflare Pages as an alternative:
"Pages gives excellent static preview URLs. It does not remove the dynamic
Rust service, database, audit store, or same-origin login boundary. A Pages
proxy would create another runtime and another place to debug cookies and
caching." — because Practice Studio is "a stateful Rust service" whose
"browser UI and API share one origin," not a static site.

The Terraform to implement this decision exists in `terraform/cloudflare/`:

- `main.tf` provisions exactly one resource type — `cloudflare_dns_record`
  CNAME records pointing at Cloudflare Tunnel targets:

  ```hcl
  resource "cloudflare_dns_record" "tunnel" {
    for_each = local.records
    zone_id  = var.zone_id
    name     = each.value.name
    type     = "CNAME"
    content  = "${each.value.tunnel_id}.cfargotunnel.com"
    proxied  = true
    ttl      = 1
  }
  ```

  Four DNS names are defined: `staging.<domain>`, `*.preview.<domain>`, and
  `ssh.staging.<domain>` all pointing at the `staging_tunnel_id`, plus
  `app.<domain>` pointing at a separate `production_tunnel_id`. This matches
  ADR 0008's design of "one wildcard preview record" plus fixed staging/
  production hostnames rather than per-PR DNS churn.
- `versions.tf` pins `cloudflare/cloudflare ~> 5.21` with an S3-compatible
  remote backend (`backend "s3" {}`, configured at `terraform init` time).
- `terraform.tfvars.example` and `variables.tf` show every value is still a
  placeholder to be filled in: `zone_id = "replace-with-zone-id"`,
  `zone_name = "example.com"`, `staging_tunnel_id = "replace-with-staging-tunnel-uuid"`,
  `production_tunnel_id = "replace-with-production-tunnel-uuid"`.
- `deploy/cloudflared/staging.yml.example` is a matching example
  `cloudflared` tunnel config, itself templated: `tunnel:
  replace-with-staging-tunnel-uuid`, ingress rules for `staging.example.com`
  -> `http://127.0.0.1:3000`, `ssh.staging.example.com` -> `ssh://127.0.0.1:22`,
  and `*.preview.example.com` -> `http://127.0.0.1:8080`, falling through to
  `http_status:404`.
- `.github/workflows/cloudflare-dns.yml` is the CI job that would apply this
  Terraform: a `workflow_dispatch` job gated by an `apply` boolean input,
  running in the `infrastructure` environment, that does `terraform init`
  against a DigitalOcean Spaces S3-compatible backend, `terraform fmt -check`,
  `terraform validate`, `terraform plan -out=dns.tfplan`, and only runs
  `terraform apply ... dns.tfplan` `if: inputs.apply`.

### The gap, stated plainly

**This path is not live.** Every value the Cloudflare Terraform and tunnel
config need — zone ID, zone name, staging tunnel UUID, production tunnel
UUID — is still the literal placeholder text from the `.example` files
(`replace-with-zone-id`, `example.com`, `replace-with-staging-tunnel-uuid`,
`replace-with-production-tunnel-uuid`). There is no evidence in this
worktree that `cloudflare-dns.yml` has ever been run with `apply: true`
against a real zone, and — the decisive proof — **`netlify.toml` still
redirects to the raw `sslip.io` IP** (`https://138-197-27-225.sslip.io/...`),
not to a `staging.<domain>` or `app.<domain>` Cloudflare Tunnel hostname. If
the Cloudflare cutover had happened, `netlify.toml`'s redirect targets would
have been updated to point at a Cloudflare-fronted hostname instead of a
sslip.io IP. They have not been. ADR 0008 is an accepted design decision;
the infrastructure to execute it exists as reviewable, plan-only Terraform;
none of it is wired into the live request path.

In short: **ADR 0008 accepted != ADR 0008 deployed.** The repo contains the
target-state Terraform and example tunnel config as a reviewed, ready-to-
apply plan, gated behind a human-approved `apply: true` dispatch — but the
actual frontend proxy target today is still the sslip.io droplet IP.

## 4. The verifier sandbox: what gates a checkpoint before Rust accepts it

Before Rust accepts a generated treatment as a checkpoint, it runs the
candidate through the `verifier/` sandbox — a Docker image built from
`verifier/Dockerfile`, referenced by the control plane via the
`WORKSPACE_VERIFIER_IMAGE` environment variable. `src/workspace_verifier.rs`
enforces that this reference is digest-pinned, not tag-only: it reads
`WORKSPACE_VERIFIER_IMAGE` and bails with `"WORKSPACE_VERIFIER_IMAGE must be
pinned by sha256 digest"` if it isn't. `verifier/README.md` gives the
expected form: `registry.example/practice-studio-verifier@sha256:...`.

### Why network is disabled

`src/workspace_verifier.rs` launches the container with an explicit,
minimal set of Docker flags, quoted directly from the run arguments:

```
"run", "--rm", "--name", &container_name,
"--network", "none",
"--read-only",
"--cpus", "1",
"--memory", "1536m", "--memory-swap", "1536m",
"--pids-limit", "256",
"--cap-drop", "ALL",
"--security-opt", "no-new-privileges",
"--user", &user,
"--tmpfs", "/tmp:rw,nosuid,nodev,noexec,size=128m",
```

`--network none` means the container has no network interface at all —
because the input to this check is *untrusted, model-influenced source*
(the Gemma-selected recipe, turned into a Svelte/Rust candidate by Rust).
The verifier's job is to run that candidate's own build/test/lint tooling
against it, and none of those tools have any legitimate reason to reach the
network during a sandboxed correctness check; disabling it removes an entire
class of exfiltration and supply-chain risk (the candidate can't phone home,
fetch unpinned dependencies at check time, or reach production). This is
reinforced inside `verifier/verify.mjs` itself: its Playwright browser smoke
check installs a request listener that treats any non-`127.0.0.1`/
`localhost` request as an error: `if (!['127.0.0.1', 'localhost'].includes(url.hostname)) errors.push(...)`.
The container is also `--read-only` with all Linux capabilities dropped
(`--cap-drop ALL`) and runs as an unprivileged, non-root user — consistent
with the same "UID/GID 65532, read-only root filesystem, all capabilities
dropped" hardening pattern the DigitalOcean runbook records for the
production/staging runtime containers.

### What it checks

`verifier/verify.mjs` defines five fixed, ordered check IDs:

```js
const CHECKS = [
  'workspace.structure.v1',
  'web.svelte-check.v1',
  'web.svelte-build.v1',
  'server.cargo-test.v1',
  'browser.synthetic-smoke.v1'
];
```

Each check short-circuits the rest on failure (later checks are recorded as
`not run: prerequisite <id> failed` rather than silently skipped), in this
order:

1. **`workspace.structure.v1`** — confirms required paths exist
   (`web/package.json`, `web/src/routes/+page.svelte`, `server/Cargo.toml`,
   `server/src/main.rs`, `synthetic`, `config/nginx.conf`, `config/start.sh`),
   that `package.json` declares `check` and `build` npm scripts, and that
   the main Svelte page actually uses a Svelte 5 rune: it fails with
   `'Svelte 5 rune $state is missing'` if `+page.svelte` doesn't contain
   `$state(`.
2. **`web.svelte-check.v1`** — runs `npm run check` in `web/` (svelte-check),
   30-second timeout.
3. **`web.svelte-build.v1`** — runs `npm run build` in `web/`, 45-second
   timeout.
4. **`server.cargo-test.v1`** — runs `cargo generate-lockfile --offline`
   then `cargo test --offline --locked --manifest-path server/Cargo.toml`,
   120-second timeout, entirely offline (no crates.io access, consistent
   with `--network none`; the Dockerfile pre-fetches dependencies at image
   build time via `cargo fetch --locked`).
5. **`browser.synthetic-smoke.v1`** — starts `vite preview` and drives a
   headless Chromium (via Playwright) to `http://127.0.0.1:4173/workspace/`,
   asserting: the response is OK, an `<h1>` exists, the page body matches
   `/synthetic/i` (the required synthetic-data warning text), `Tab` moves
   keyboard focus off `<body>` (basic accessibility), and zero page/console
   errors were captured — plus the non-localhost request check described
   above. 20-second timeout.

The report is written as one bounded JSON document (`{ checks: [...] }`)
to a path *inside* the workspace mount, and the process exits non-zero if
any check failed. `src/workspace_verifier.rs` then validates the check
order and computes evidence digests over that report — so Rust, not the
verifier image itself, is the final arbiter of whether a checkpoint is
accepted.

## Summary: live vs. staged, at a glance

| Layer | Status | Evidence |
| --- | --- | --- |
| Netlify static host + redirects | **Live** | `netlify.toml` redirects to `https://138-197-27-225.sslip.io/...` |
| DigitalOcean single staging droplet | **Live** | `docs/digitalocean-runbook.md` "Proven baseline" 2026-07-12 |
| `staging-preview.yml` firewall-punch + deploy + PR comment | **Live** (manually dispatched) | `.github/workflows/staging-preview.yml` |
| Native Rust + DigitalOcean Gemma planner | **Live** | `src/workspace_agent.rs`, `src/workspace.rs`, ADR 0009, runbook's `practice-studio-treatment-planner` |
| Python `services/agent/` microservice | **Removed** | `git log --diff-filter=D -- services/agent` -> commit `0bccc87` |
| Cloudflare DNS/tunnel routing (ADR 0008) | **Accepted design, NOT cut over** | `terraform/cloudflare/*.tfvars.example` still placeholder values; `netlify.toml` still targets sslip.io, not a Cloudflare hostname |
| `verifier/` sandboxed check pipeline | **Live**, gates every checkpoint | `src/workspace_verifier.rs` enforces digest pin + `--network none`; `verifier/verify.mjs` five ordered checks |
