# Terraform

Terraform, OpenTofu and Terragrunt files share one language.

```toml
[languages.terraform]      # off unless the table is written
include = ["**/*.tf", "**/*.tofu", "**/terragrunt.hcl"]   # the default
ignore_unresolved = ["../../shared/*"]   # module sources not to warn about
```

| Option | |
|---|---|
| `include` | Globs of the files to scan. A `.hcl` file that it matches is read as a Terragrunt file. |
| `ignore_unresolved` | Name patterns of the module sources and dependency paths that are expected not to resolve, so are not reported as unresolved. |

The `.terraform` and `.terragrunt-cache` directories, where Terraform and
Terragrunt keep the modules they download, are never scanned.

## Packages

Terraform has no manifest that names a package, so its projects have no
packages, and `pkg:` selects none of their directories. Name a module by its
directory instead, such as `modules/vpc`.

## Imports

A module is a directory, so an import of a module of the project is an import
of its directory, which is one node of the graph, and a contract names
`modules/vpc` and not the files in it. Every import is a `value` import, and is
located by its string:

| File | Import |
|---|---|
| `.tf`, `.tofu` | The `source` of each `module` block. |
| `terragrunt.hcl` | The `source` of the `terraform` block, the `config_path` of each `dependency` block, and each of the `paths` of the `dependencies` block. |

A value made from an expression, such as
`"${get_repo_root()}/modules/vpc"` or `local.source`, can't be read without
evaluating it, so it is left out of the graph. `include` blocks and the other
attributes of a block are not imports.

[`zenith-ignore` comments](../suppressions.md) are `#`, `//` or `/* */`
comments. One on the line where a block or an attribute starts, or on the line
above it, applies to every import in it:

```hcl
# zenith-ignore stacks-independent: shares the production database until the split
module "database" {
  source = "../prod"
}
```

## Resolution

| Imported | Node | |
|---|---|---|
| a local path, such as `../../modules/vpc` | `modules/vpc` | The directory it leads to from the importing file's directory, which must be in the project. A `//` before a subdirectory, as in `../modules//vpc`, leads to the subdirectory. |
| a registry module, such as `hashicorp/consul/aws` | `tfmod:hashicorp/consul/aws` | Named by its namespace, name and target system. A private registry's hostname stays in front, as in `tfmod:app.terraform.io/acme/vpc/aws`. Terragrunt's `tfr://` addresses are read the same. |
| a repository, a bucket or an archive, such as `git::https://github.com/acme/modules.git//vpc?ref=v1.2.0` | `tfmod:github.com/acme/modules` | Named by its host and path, without the getter, scheme, credentials, port, `.git`, subdirectory or query. On GitHub and Bitbucket, written without a scheme, the repository is the first two segments of the path, so `github.com/acme/modules/vpc` is `tfmod:github.com/acme/modules` too. |

Registry names and hostnames ignore case, so both are in lowercase.

Each file resolves as its tool reads it:

- In a Terraform file, as Terraform requires, a local path starts with `./`
  or `../`, so `modules/vpc` is not a module source, and is reported as
  unresolved. Anything else that is not a repository is a registry address.
- In a Terragrunt file, a registry module is written with `tfr://`, and
  anything else that is not a repository is a local path, so
  `config_path = "vpc"` is the directory `vpc` next to the file.
