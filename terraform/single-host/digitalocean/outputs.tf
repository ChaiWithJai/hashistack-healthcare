output "public_ip" { value = digitalocean_droplet.studio.ipv4_address }
output "studio_url" { value = "http://${digitalocean_droplet.studio.ipv4_address}:3000" }
output "release_ref" { value = var.release_ref }
