provider "google" {
  project = var.project
  region  = var.region
  zone    = var.zone
}
resource "random_password" "postgres" {
  length  = 32
  special = false
}

resource "google_compute_address" "studio" { name = "hashistack-healthcare-studio" }
resource "google_compute_instance" "studio" {
  name         = "hashistack-healthcare-studio"
  machine_type = var.machine_type
  tags         = ["hashistack-studio"]
  boot_disk {
    initialize_params {
      image = "ubuntu-os-cloud/ubuntu-2404-lts-amd64"
      size  = 30
    }
  }
  network_interface {
    network = "default"
    access_config { nat_ip = google_compute_address.studio.address }
  }
  metadata = {
    ssh-keys = "${var.ssh_user}:${var.ssh_public_key}"
    user-data = templatefile("${path.module}/../cloud-init.yaml.tftpl", {
      postgres_password = random_password.postgres.result
      repository_url    = var.repository_url
      release_ref       = var.release_ref
    })
  }
}
resource "google_compute_firewall" "studio_web" {
  name          = "hashistack-healthcare-studio-web"
  network       = "default"
  target_tags   = ["hashistack-studio"]
  source_ranges = var.studio_cidrs
  allow {
    protocol = "tcp"
    ports    = ["3000"]
  }
}
resource "google_compute_firewall" "studio_ssh" {
  name          = "hashistack-healthcare-studio-ssh"
  network       = "default"
  target_tags   = ["hashistack-studio"]
  source_ranges = var.admin_cidrs
  allow {
    protocol = "tcp"
    ports    = ["22"]
  }
}
