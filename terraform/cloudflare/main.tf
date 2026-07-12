locals {
  records = {
    staging = {
      name      = "staging.${var.zone_name}"
      tunnel_id = var.staging_tunnel_id
    }
    preview = {
      name      = "*.preview.${var.zone_name}"
      tunnel_id = var.staging_tunnel_id
    }
    production = {
      name      = "app.${var.zone_name}"
      tunnel_id = var.production_tunnel_id
    }
  }
}

resource "cloudflare_dns_record" "tunnel" {
  for_each = local.records

  zone_id = var.zone_id
  name    = each.value.name
  type    = "CNAME"
  content = "${each.value.tunnel_id}.cfargotunnel.com"
  proxied = true
  ttl     = 1
  comment = "Practice Studio ${each.key}; managed by Terraform"
}
