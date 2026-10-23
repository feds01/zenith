### The messages about Terraform and Terragrunt: the syntax of their files,
### and how their module sources resolve.

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

## Why a module source or a dependency path does not resolve.

terraform_no_directory = no directory `{$dir}` in the project
terraform_not_an_address = not a module source that Terraform reads: a local path starts with `./` or `../`
