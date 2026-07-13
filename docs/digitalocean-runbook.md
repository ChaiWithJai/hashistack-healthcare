# DigitalOcean deployment runbook

This runbook turns the portable synthetic studio into a repeatable DigitalOcean proof. It is not authorization to process PHI. Keep all prompts, fixtures, logs, and exports synthetic until a BAA is executed and every service in the data path is confirmed eligible.

## Proven baseline

On 2026-07-12 the `single-host` module provisioned Ubuntu 24.04 in `nyc3` on `s-2vcpu-4gb` (2 vCPU, 4 GiB, 80 GiB; $24/month before backups). The module enabled backups, IPv6, DigitalOcean monitoring, and a cloud firewall restricted to the operator's `/32` for SSH and port 3000.

Measured from the operator's machine after boot:

| Check | Result |
| --- | --- |
| Terraform apply | Droplet, firewall, and generated database password created |
| Cloud-init | `done`, no reported errors |
| First boot to ready | about 7 minutes |
| Rust release build within first boot | 3m38s |
| Runtime hardening | UID/GID 65532, read-only root filesystem, all capabilities dropped |
| Persistence | 2 apps before and after control-plane restart |
| Health latency | p50 33.1 ms, p95 39.5 ms over 30 requests |
| Authenticated app-list latency | p50 33.7 ms, p95 38.6 ms over 30 requests |
| Idle memory | studio 1.2 MiB, Postgres 72 MiB |

The source build is acceptable as a portability fallback, but too slow for a production recovery path. Builder and runtime bases are digest-pinned and BuildKit caches Cargo dependencies between host builds; production must still publish the reviewed image once in CI and deploy that result by immutable digest before treating recovery time as production-ready.

Cloud-init is intentionally first-boot only. Updating `release_ref` does not
replace the Droplet or erase its volumes. Advance an existing host with the
full reviewed commit SHA and rerun the provider-neutral proof:

```sh
scripts/single-host-release.sh root@203.0.113.10 <40-character-commit-sha>
```

## Local prerequisites

1. Install `doctl` and Terraform.
2. Create a short-lived DigitalOcean token. Terraform needs droplet, firewall, SSH-key, VPC, project, tag, image, region, size, and monitoring lifecycle permissions.
3. Register an SSH key with DigitalOcean.
4. Find the operator's current public IP and use a `/32`. Do not make SSH public.
5. Pin `release_ref` to a reviewed commit or tag, never a moving branch.

Terraform state contains the generated Postgres password. It is ignored by Git, but local state is still a secret. A team deployment must use an encrypted remote state backend with locking and a documented recovery owner.

## Provision and prove

```sh
cd terraform/single-host/digitalocean
terraform init
terraform plan -out=proof.tfplan \
  -var 'ssh_key_fingerprint=…' \
  -var 'admin_cidrs=["203.0.113.4/32"]' \
  -var 'studio_cidrs=["203.0.113.4/32"]' \
  -var 'release_ref=<reviewed-commit>'
terraform apply proof.tfplan

cd ../../..
scripts/single-host-remote-proof.sh "$(terraform -chdir=terraform/single-host/digitalocean output -raw studio_url)"
```

On the host, require all of these:

```sh
cloud-init status --long
systemctl is-active docker do-agent
cd /opt/hashistack-healthcare
docker compose --env-file /etc/hashistack-studio.env ps
docker inspect hashistack-healthcare-studio-studio-1 \
  --format 'user={{.Config.User}} readonly={{.HostConfig.ReadonlyRootfs}} capdrop={{json .HostConfig.CapDrop}}'
```

Restart the studio container, rerun the remote proof, and confirm earlier apps and audit events remain. A successful `terraform apply` alone is not deployment proof.

## One Droplet for staging and production

A shared Droplet is an explicit hobby-MVP cost trade-off, not high availability. Run two Compose projects (`staging` and `prod`) with separate networks, Postgres databases, volumes, identity files, audit keys, environment files, resource limits, and loopback ports. Put one TLS proxy in front and route distinct hostnames. Never mount the Docker socket into either application.

This isolates ordinary configuration and data mistakes, but both environments still share a kernel, disk, network, maintenance window, and failure domain. The promotion path is an immutable image digest: staging proves the digest, then production adopts that exact digest. Do not rebuild between environments.

The next production topology is two Droplets (or App Platform services), separate databases, and a load balancer/TLS boundary. Managed Postgres becomes worthwhile when backup/restore testing, point-in-time recovery, and operator separation matter more than the single-host cost saving.

## Agent and model tier

The staging planner is a private DigitalOcean agent named
`practice-studio-treatment-planner` in `tor1`. It uses Gemma 4. The release
workflow copies its endpoint, scoped key, and immutable version into
`/etc/hashistack-studio.env`, then the remote proof requires the returned
workspace to report that exact provider, model, and version with no fallback.
Configure the GitHub `staging` environment with:

- variable `DIGITALOCEAN_PLANNER_ENDPOINT`;
- variable `DIGITALOCEAN_PLANNER_VERSION`;
- secret `DIGITALOCEAN_PLANNER_ACCESS_KEY`;
- secret `DO_STAGING_HOST`, plus the existing SSH key and known-hosts secrets.

Hosted GitHub runners do not have a stable source address. The protected
staging job uses `DO_STAGING_FIREWALL_TOKEN` and
`DO_STAGING_FIREWALL_ID` to add one exact runner `/32` for TCP 22. A shell
trap removes the same rule whether deployment passes or fails. The token needs
only firewall update access. SSH still requires the staging private key and a
pinned known-host entry; ports 80 and 443 remain the public application path.

Gemma 4 is the only application model. It chooses one of three recipes from the
signed pack. Rust rejects any other answer. Rust creates the candidate source,
runs the checks, stores the accepted checkpoint, and creates the export. The
repository does not deploy a second model worker or an agent framework.

Use `prompts/digitalocean-treatment-planner-v1.txt` as the exact DigitalOcean
agent instruction text. After any change, hash that file and store the digest
in `DIGITALOCEAN_PLANNER_VERSION`. Deploy the same digest with the application.

The DigitalOcean agent must receive synthetic data only. Do not send patient
data to the agent until DigitalOcean confirms the required health care terms
and the full data retention path in writing.

The MVP uses one small CPU Droplet for the application services and the private
DigitalOcean Gemma agent for planning. It does not need Kubernetes. Add more
hosts only when the measured load or the required isolation calls for them.

## Rollback and teardown

Rollback production by restoring the previously proven image digest and rerunning the remote proof. Database rollback means restoring a tested backup into a new database and switching deliberately; do not overwrite the only copy.

For a disposable proof:

```sh
terraform -chdir=terraform/single-host/digitalocean destroy
```

Before destroying a durable environment, export owned app bundles, the append-only audit stream, and a database backup; verify the restore procedure and record who approved deletion.
