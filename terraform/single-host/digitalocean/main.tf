provider "digitalocean" {}

resource "random_password" "postgres" {
  length  = 32
  special = false
}

resource "digitalocean_droplet" "studio" {
  name     = "hashistack-healthcare-studio"
  image    = "ubuntu-24-04-x64"
  region   = var.region
  size     = var.size
  ssh_keys = [var.ssh_key_fingerprint]
  tags     = ["hashistack-studio", "synthetic-only"]
  user_data = templatefile("${path.module}/../cloud-init.yaml.tftpl", {
    postgres_password = random_password.postgres.result
    repository_url    = var.repository_url
    release_ref       = var.release_ref
  })
}

resource "digitalocean_firewall" "studio" {
  name        = "hashistack-healthcare-studio"
  droplet_ids = [digitalocean_droplet.studio.id]
  inbound_rule {
    protocol         = "tcp"
    port_range       = "22"
    source_addresses = var.admin_cidrs
  }
  inbound_rule {
    protocol         = "tcp"
    port_range       = "3000"
    source_addresses = var.studio_cidrs
  }
  outbound_rule {
    protocol              = "tcp"
    port_range            = "1-65535"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
  outbound_rule {
    protocol              = "udp"
    port_range            = "53"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
}
