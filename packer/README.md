# Packer status

Packer is the selected tool for a versioned DigitalOcean host image. The
minimum lovable version does not yet have an observed Packer host replacement
proof.

The current `control-plane.pkr.hcl` and `client.pkr.hcl` files describe an
older Nomad and Vault reference architecture. They are not executable setup
instructions for the current Docker Compose runtime. Do not use them to create
the minimum lovable host.

The supported provisioning path is in `terraform/single-host/`. Cloud init
installs Docker and runs the root Compose file. A later pull request can add a
single host Packer template when the release gate also measures replacement
time, records the snapshot identifier, and proves the same exact image digest
with `scripts/single-host-remote-proof.sh`.
