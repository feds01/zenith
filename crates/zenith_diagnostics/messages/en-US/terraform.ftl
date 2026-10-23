### The messages about Terraform and Terragrunt: the syntax of their files.

## What a reference to another module is, by where it is written.

terraform_module_source = module source
terraform_terragrunt_source = Terraform source
terraform_dependency_path = dependency path

## What is wrong in a syntax error of a Terraform or Terragrunt file. A kind
## of node is named in words, such as "block end", and a reference is one of
## the names above.

terraform_source_too_large = the source is too large to read
terraform_invalid_syntax = invalid syntax
terraform_expected_node = expected {$node}
terraform_expected_token = expected `{$token}`
terraform_escaped_reference = escapes in a {$reference} are not supported
terraform_empty_reference = the {$reference} is empty
