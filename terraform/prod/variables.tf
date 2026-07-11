variable "control_plane_image_id" {
  description = "Packer-built control-plane image (Nomad server + Vault). A security patch is a new image and a rolling replace, never an ssh session."
  type        = string
}

variable "client_image_id" {
  description = "Packer-built client image (Nomad client, task drivers, hipaa-core runtime deps)."
  type        = string
}

variable "sandbox_pool_size" {
  description = "Nomad clients tagged role=sandbox — preview allocations, no route to tenant databases."
  type        = number
  default     = 2
}

variable "prod_pool_size" {
  description = "Nomad clients tagged role=prod — promoted apps only."
  type        = number
  default     = 2
}

variable "ingress_certificate_name" {
  description = "DO-managed certificate for the single ingress load balancer."
  type        = string
}
