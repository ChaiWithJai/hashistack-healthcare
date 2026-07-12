output "public_ip" { value = google_compute_address.studio.address }
output "studio_url" { value = "http://${google_compute_address.studio.address}:3000" }
output "release_ref" { value = var.release_ref }
