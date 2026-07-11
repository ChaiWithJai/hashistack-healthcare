# Control-plane image family: Nomad server + Vault (Raft storage).
# Three of these make a quorum; they never run tenant workloads.

packer {
  required_plugins {
    digitalocean = {
      source  = "github.com/digitalocean/digitalocean"
      version = ">= 1.0.0"
    }
  }
}

variable "image_version" {
  type        = string
  description = "Immutable image version, e.g. v2026.07.1."
}

source "digitalocean" "control_plane" {
  region        = "nyc3"
  size          = "s-1vcpu-2gb"
  image         = "debian-12-x64"
  snapshot_name = "clinician-control-plane-${var.image_version}"
  ssh_username  = "root"
  tags          = ["clinician-platform", "image:control-plane"]
}

build {
  sources = ["source.digitalocean.control_plane"]

  provisioner "shell" {
    scripts = [
      "scripts/install-nomad-server.sh",
      "scripts/install-vault.sh", # raft storage; audit device enabled at init
      "scripts/harden.sh",
    ]
    environment_vars = ["IMAGE_VERSION=${var.image_version}"]
  }
}
