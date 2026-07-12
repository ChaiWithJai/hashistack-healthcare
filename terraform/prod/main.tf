# Infrastructure layer: DigitalOcean under BAA, one VPC per environment.
# Terraform owns everything here — nothing is click-ops; the audit trail is
# this file's git history plus the Vault audit log.
#
# BAA scope note (RFC): DO signs BAAs on covered products only. Everything
# that can touch PHI stays inside covered products: Droplets, managed
# Postgres, Spaces, Load Balancers. Verify the current covered list before
# contract.

terraform {
  required_version = ">= 1.7"
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "~> 2.0"
    }
  }
  # One state per environment, in CI-managed remote state.
  backend "s3" {} # DO Spaces, configured via -backend-config in CI
}

provider "digitalocean" {}

locals {
  env    = "prod"
  region = "nyc3"
}

resource "digitalocean_vpc" "env" {
  name   = "clinician-platform-${local.env}"
  region = local.region
}

# --- control plane pool: 3 small droplets, Nomad servers + Vault (Raft).
# Odd count for quorum. These never run tenant workloads.
resource "digitalocean_droplet" "control" {
  count    = 3
  name     = "control-${local.env}-${count.index}"
  region   = local.region
  size     = "s-2vcpu-4gb"
  image    = var.control_plane_image_id # Packer-built, immutable
  vpc_uuid = digitalocean_vpc.env.id
  tags     = ["clinician-platform", local.env, "role:control"]
}

# --- sandbox pool: untrusted generated code, preview allocations only.
resource "digitalocean_droplet" "sandbox" {
  count    = var.sandbox_pool_size
  name     = "sandbox-${local.env}-${count.index}"
  region   = local.region
  size     = "s-4vcpu-8gb"
  image    = var.client_image_id # Packer-built, immutable
  vpc_uuid = digitalocean_vpc.env.id
  tags     = ["clinician-platform", local.env, "role:sandbox"]
}

# --- prod pool: promoted apps only; the only pool with tenant db access.
resource "digitalocean_droplet" "prod" {
  count    = var.prod_pool_size
  name     = "prod-${local.env}-${count.index}"
  region   = local.region
  size     = "s-4vcpu-8gb"
  image    = var.client_image_id
  vpc_uuid = digitalocean_vpc.env.id
  tags     = ["clinician-platform", local.env, "role:prod"]
}

# --- tenant data: managed Postgres (covered product), one logical database
# per tenant app, created by the deploy service via the admin connection.
resource "digitalocean_database_cluster" "tenant_pg" {
  name       = "tenant-pg-${local.env}"
  engine     = "pg"
  version    = "16"
  size       = "db-s-2vcpu-4gb"
  region     = local.region
  node_count = 2
  private_network_uuid = digitalocean_vpc.env.id
}

# The prod pool is the ONLY compute allowed to reach tenant Postgres.
# This firewall is the sandbox/prod split as code — a compliance control,
# not just a network preference.
resource "digitalocean_database_firewall" "tenant_pg_prod_only" {
  cluster_id = digitalocean_database_cluster.tenant_pg.id

  rule {
    type  = "tag"
    value = "role:prod"
  }
}

# --- uploads: Spaces with SSE; field-level envelope encryption comes from
# hipaa-core via Vault transit on top of this.
resource "digitalocean_spaces_bucket" "uploads" {
  name   = "clinician-uploads-${local.env}"
  region = local.region
  acl    = "private"
}

# --- single ingress path: LB → router system job on the client pools.
resource "digitalocean_loadbalancer" "ingress" {
  name     = "ingress-${local.env}"
  region   = local.region
  vpc_uuid = digitalocean_vpc.env.id

  forwarding_rule {
    entry_port      = 443
    entry_protocol  = "https"
    target_port     = 8080
    target_protocol = "http"
    certificate_name = var.ingress_certificate_name
  }

  healthcheck {
    port     = 8080
    protocol = "http"
    path     = "/health"
  }

  droplet_tag = "role:prod"
}

# Droplet firewall: private-by-default; only the LB reaches the pools.
resource "digitalocean_firewall" "pools" {
  name = "clinician-pools-${local.env}"
  tags = ["role:sandbox", "role:prod"]

  inbound_rule {
    protocol                  = "tcp"
    port_range                = "8080"
    source_load_balancer_uids = [digitalocean_loadbalancer.ingress.id]
  }

  inbound_rule {
    protocol    = "tcp"
    port_range  = "4646-4648" # nomad
    source_tags = ["role:control"]
  }

  outbound_rule {
    protocol              = "tcp"
    port_range            = "1-65535"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
}
