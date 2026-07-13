# Deploy Practice Studio

The minimum lovable version has one supported application runtime. Docker
Compose runs the Rust Studio service and Postgres on one DigitalOcean Droplet.
Caddy provides TLS and routing. A verifier container starts only when source
verification is requested.

Practice Studio is a synthetic learning environment. This deployment is not
approved for patient data or clinical care.

## Environments

| Environment | Frontend | API | Data |
|---|---|---|---|
| Local | Rust serves the checked out client | Local Docker Compose | Synthetic only |
| Pull request | Netlify builds the exact pull request frontend | DigitalOcean staging | Synthetic only |
| Staging | Staging frontend | DigitalOcean staging Compose project | Synthetic only |
| Production candidate | Owned frontend | Separate Compose project or host | Synthetic only until every production control is proved |

The pull request preview is for product review. It must report the exact
frontend commit. The staging proof must report the exact Rust commit. A green
frontend preview does not prove the backend, and a green backend proof does not
prove the browser flow.

Set `WORKSPACE_AGENT_TIMEOUT_SECS=20` on the staging host. Netlify can close a
slow proxied request before a longer model timeout finishes. Rust returns the
signed local treatment set after 20 seconds and records the fallback. Run the
Gemma provider profile directly against staging to prove a valid provider
response without the proxy.

## One host boundary

A hobby deployment may run staging and a production candidate on one Droplet.
Use separate Compose project names, networks, Postgres databases, volumes,
environment files, audit keys, and loopback ports. Both environments still
share a kernel, disk, network, and maintenance window.

Use separate hosts before accepting an uptime goal or any workload that can
receive patient data.

## Build and release

GitHub checks the exact source commit. CI builds the Studio image and the
executable verifier image. Record each image by SHA 256 digest. Staging proves
the digest before another environment can use it. Do not rebuild between
environments.

The verifier image must be executable. The verifier runs with networking
disabled and with fixed limits. Set its concurrency to one on the 4 GB host. A
concurrent request must fail without starting a second container.

Packer will create a versioned DigitalOcean host image when the release gate
includes a measured host replacement time. Until that work is proved, the
current cloud init source build is a portability fallback and not an immutable
recovery path.

## Identity

The build flow works without login. Configure separate Clerk development and
production instances. Ask for identity only when a doctor claims or exports a
workspace. A Netlify preview host cannot claim or export a workspace unless it
is an authorized Clerk party.

## Required configuration

The protected `staging` GitHub environment needs:

- `DO_STAGING_HOST`;
- `DO_STAGING_URL`;
- `DO_STAGING_SSH_KEY`;
- `DO_STAGING_KNOWN_HOSTS`;
- `DIGITALOCEAN_PLANNER_ENDPOINT`;
- `DIGITALOCEAN_PLANNER_VERSION`;
- `DIGITALOCEAN_PLANNER_ACCESS_KEY`;
- the Clerk development values;
- the Postgres password and audit keys;
- the verifier image digest.

Use a firewall rule that grants the GitHub runner access for the deployment
and removes that access when the job ends. Keep ports 80 and 443 as the public
application path.

## Verify

Run the provider neutral proof against the exact deployed commit:

```bash
curl -fsS https://<host>/health
scripts/single-host-remote-proof.sh https://<host>
```

The proof must create an anonymous workspace, accept a bounded treatment,
reject a patient data release, publish a synthetic preview, preserve state
after restart, and reject anonymous export.

Then complete the browser checklist from
[Try the hosted preview](get-started/hosted-preview.md).

## Rollback and teardown

Rollback selects an earlier image digest that already passed staging. A
database rollback restores a tested backup into a new database. Do not
overwrite the only copy.

Destroy a disposable host with:

```bash
terraform -chdir=terraform/single-host/digitalocean destroy
```

Read the [DigitalOcean runbook](digitalocean-runbook.md) for the measured cost,
provisioning steps, and limits. Read
[decision 0010](decisions/0010-minimum-lovable-runtime.md) for the architecture
boundary.
