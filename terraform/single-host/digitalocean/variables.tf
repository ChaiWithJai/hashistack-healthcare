variable "ssh_key_fingerprint" {
  description = "Fingerprint of an SSH key already registered in DigitalOcean."
  type        = string
}
variable "admin_cidrs" {
  description = "CIDRs allowed to reach SSH. Never use 0.0.0.0/0 for a persistent host."
  type        = list(string)
}
variable "studio_cidrs" {
  description = "CIDRs allowed to reach the synthetic studio. Use 0.0.0.0/0 only for a disposable public demo containing no patient data."
  type        = list(string)
}
variable "region" {
  type    = string
  default = "nyc3"
}
variable "size" {
  type    = string
  default = "s-2vcpu-4gb"
}
variable "repository_url" {
  type    = string
  default = "https://github.com/ChaiWithJai/hashistack-healthcare.git"
}
variable "release_ref" {
  description = "Reviewed immutable tag or commit containing the single-host bundle."
  type        = string
}
