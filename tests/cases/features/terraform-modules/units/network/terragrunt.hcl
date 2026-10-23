terraform {
  source = "../../modules//vpc"
}

inputs = {
  cidr = "10.1.0.0/16"
}
