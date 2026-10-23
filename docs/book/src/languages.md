# Languages

zenith reads JavaScript and TypeScript, Go, Python, Rust and Terraform.
Contracts, selectors, the baseline and the output work the same in every
language: each language only decides which files it reads, which packages a
project has, and what each import refers to. Each has a page of its own:

| Language | Table | Files | Packages | External packages | Builtins |
|---|---|---|---|---|---|
| [JavaScript and TypeScript](languages/javascript.md) | `[languages.js]` | `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs`, `.cjs` | workspace packages | `npm:react` | `node:fs`, `bun:sqlite` |
| [Go](languages/go.md) | `[languages.go]` | `.go` | modules | `gomod:github.com/pkg/errors` | `go:net/http` |
| [Python](languages/python.md) | `[languages.python]` | `.py`, `.pyi` | packages with a `pyproject.toml` | `pypi:requests` | `python:os` |
| [Rust](languages/rust.md) | `[languages.rust]` | `.rs` | crates | `crate:serde` | `rust:std` |
| [Terraform](languages/terraform.md) | `[languages.terraform]` | `.tf`, `.tofu`, `terragrunt.hcl` | none | `tfmod:hashicorp/consul/aws` | none |

## Enabling languages

Each language has a table of its own in `[languages]`, with the options that
its page lists, and a language is enabled by writing its table, even an empty
one:

```toml
[languages.js]
[languages.go]
[languages.python]
[languages.rust]
[languages.terraform]
```

Without a `[languages]` table, JavaScript and TypeScript are enabled. A
`[languages]` table with no language in it is an error, since there would be
nothing to scan. Several languages can be enabled at once, and their files
share one import graph, so a contract can span them.

## Files

Each language scans the files that its `include` globs match, which default to
its own source files. A file belongs to the first language whose `include`
matches it, or, for a language that leaves `include` to its default, whose
extensions or file names it has, such as Terraform's `terragrunt.hcl`. The
[`[files]`](configuration.md#files) table is about what is skipped, whatever
the language.

A path selector may leave the extension off a file, so `src/types` matches the
file `src/types.ts` as well as the directory `src/types`. The extensions it
may leave off are those of the enabled languages.

## Schemes

A language owns the selector schemes that only its projects have, such as
`npm:` for JavaScript and `gomod:` for Go, so a selector that uses one is an
error when its language is not enabled. `pkg:` selects the packages of any
language, and belongs to none.
