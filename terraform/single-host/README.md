# Single-host synthetic studio

This profile proves one interoperable doctor workflow on a laptop, a
DigitalOcean droplet, or a GCP VM: describe, customize, publish with synthetic
data, restart, and export an owned bundle. Both providers render the same
cloud-init template and run the root Compose file.

It is intentionally **not** the managed PHI profile. It has one host, static
Phase-0 identities, no TLS, and no Nomad/Vault workload identity. Never place
patient data in it. `terraform/prod/` remains the future multi-node managed
substrate; `scripts/staging-docker-up.sh` is the local HashiStack integration
proof.

Local proof:

```sh
scripts/single-host-smoke.sh
docker compose down                 # retains state
docker compose down --volumes       # destructive reset
```

DigitalOcean:

```sh
cd terraform/single-host/digitalocean
terraform init
terraform apply \
  -var 'ssh_key_fingerprint=…' \
  -var 'admin_cidrs=["203.0.113.4/32"]' \
  -var 'studio_cidrs=["203.0.113.4/32"]' \
  -var 'release_ref=716e3f6146644c59616ae8f309dd0dd9b544f426'
```

GCP uses the sibling `gcp/` module with `project`, `ssh_user`,
`ssh_public_key`, and `admin_cidrs`. Pin `release_ref` to a reviewed tag or
commit for a durable deployment. A public `studio_cidrs=["0.0.0.0/0"]` is an
explicit disposable-demo choice; never combine it with patient data or the
tracked development bearer tokens.
