variable "project" { type = string }
variable "region" {
  type    = string
  default = "us-east1"
}
variable "zone" {
  type    = string
  default = "us-east1-b"
}
variable "machine_type" {
  type    = string
  default = "e2-medium"
}
variable "admin_cidrs" { type = list(string) }
variable "studio_cidrs" { type = list(string) }
variable "ssh_user" { type = string }
variable "ssh_public_key" {
  type      = string
  sensitive = true
}
variable "repository_url" {
  type    = string
  default = "https://github.com/ChaiWithJai/hashistack-healthcare.git"
}
variable "release_ref" { type = string }
