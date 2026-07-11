# Client image family: Nomad client + task drivers + hipaa-core runtime deps.
# Images are versioned and immutable. A patch is a new image + rolling
# replace — there is deliberately no ssh provisioning path after bake time.

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
  description = "Immutable image version, e.g. v2026.07.1 — surfaces in every allocation and audit event."
}

source "digitalocean" "client" {
  region        = "nyc3"
  size          = "s-1vcpu-2gb"
  image         = "debian-12-x64"
  snapshot_name = "clinician-client-${var.image_version}"
  ssh_username  = "root"
  tags          = ["clinician-platform", "image:client"]
}

build {
  sources = ["source.digitalocean.client"]

  provisioner "shell" {
    scripts = [
      "scripts/install-nomad-client.sh", # nomad client + docker task driver
      "scripts/install-hipaa-core.sh",   # runtime deps for generated apps
      "scripts/harden.sh",               # CIS-ish baseline, auditd, no password ssh
    ]
    environment_vars = ["IMAGE_VERSION=${var.image_version}"]
  }
}
