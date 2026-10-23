include "root" {
  path = find_in_parent_folders("root.hcl")
}

terraform {
  source = "tfr:///acme/app/aws?version=1.0.0"
}

dependency "network" {
  config_path = "../network"
}

inputs = {
  vpc_id = dependency.network.outputs.id
}
