# The production stack.
terraform {
  required_providers {
    aws = {
      source = "hashicorp/aws"
    }
  }
}

module "vpc" {
  source = "../../modules/vpc"
  cidr   = "10.0.0.0/16"
}

module "app" {
  source = "../../modules/app"
  vpc_id = module.vpc.id
}

module "consul" {
  source  = "hashicorp/consul/aws"
  version = "0.1.0"
}

module "dns" {
  source = "git::https://github.com/acme/modules.git//dns?ref=v1.2.0"
}
