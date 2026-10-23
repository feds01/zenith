variable "vpc_id" {
  type = string
}

module "settings" {
  source = "../../live/prod"
}

module "database" {
  source = "../rds"
}
