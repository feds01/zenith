module "vpc" {
  source = "../../modules/vpc"
}

# zenith-ignore stacks-independent: shares the production database until the split
module "database" {
  source = "../prod"
}
