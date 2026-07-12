variable "zone_id" {
  description = "Cloudflare zone ID for the owned domain."
  type        = string
}

variable "zone_name" {
  description = "Owned DNS zone, for example example.com."
  type        = string
}

variable "staging_tunnel_id" {
  description = "UUID of the staging and preview Cloudflare Tunnel."
  type        = string
}

variable "production_tunnel_id" {
  description = "UUID of the production Cloudflare Tunnel."
  type        = string
}
