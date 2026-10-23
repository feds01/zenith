# Go

```toml
[languages.go]             # off unless the table is written
include = ["**/*.go"]      # the default
ignore_unresolved = ["example.com/generated/*"]   # import paths not to warn about
```

| Option | |
|---|---|
| `include` | Globs of the files to scan. |
| `ignore_unresolved` | Name patterns of the import paths that are expected not to resolve, so are not reported as unresolved. |

## Packages

The project's modules are found from its `go.mod` and `go.work` files. Each is
named by its module path, so `pkg:example.com/api` selects a module.

## Imports

Every import is a `value` import. An import of a package in the project is an
import of its directory, which is one node of the graph, so a contract names
`internal/store` and not the files in it.
[`zenith-ignore` comments](../suppressions.md) work among a file's imports, as
`//` or `/* */` comments.

## Resolution

Import paths resolve against the modules that the `go.mod` and `go.work` files
declare.

| Imported | Node | |
|---|---|---|
| a package of the project | `internal/store` | Its directory. |
| the standard library | `go:net/http` | The whole import path. |
| any other module | `gomod:github.com/pkg/errors` | Named by the module that provides it. A module that no `go.mod` requires is named from the import path alone. |
