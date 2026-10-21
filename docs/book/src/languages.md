# Languages

zenith reads JavaScript and TypeScript, and Go. Contracts, selectors, the
baseline and the output work the same in every language: each language only
decides which files it reads, which packages a project has, and what each
import refers to.

A language is enabled by writing its table in [`[languages]`](configuration.md#languages).
Without a `[languages]` table, JavaScript and TypeScript are enabled. Several
languages can be enabled at once, and their files share one import graph.

| Language | Table | Files | Packages | External packages | Builtins |
|---|---|---|---|---|---|
| [JavaScript and TypeScript](#javascript-and-typescript) | `[languages.js]` | `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs`, `.cjs` | workspace packages | `npm:react` | `node:fs`, `bun:sqlite` |
| [Go](#go) | `[languages.go]` | `.go` | modules | `gomod:github.com/pkg/errors` | `go:net/http` |

A file belongs to the first language whose `include` globs match it, or, for a
language that leaves `include` to its default, whose extensions it has.

## JavaScript and TypeScript

```toml
[languages.js]             # also accepted as [languages.typescript]
```

**Packages.** Workspace packages are found from the root `package.json`'s
`workspaces` field and from `pnpm-workspace.yaml`. Each has a name, from its
own `package.json`, and a directory, so `pkg:@acme/ui` selects the package
named `@acme/ui` wherever it lives.

**Imports.** Each import statement is recorded with its kind:

| Kind | Made by |
|---|---|
| `value` | `import x from "s"`, `import "s"`, `export … from "s"` |
| `type` | `import type …`, `export type …`, imports whose every name is `type`, and TypeScript import types such as `typeof import("s")` |
| `dynamic` | `import("s")` with a literal specifier |
| `require` | `require("s")` with a literal specifier, and `import x = require("s")` |

Imports whose specifier isn't a literal, such as `import(path)`, are skipped
and counted; `-v` lists them. A file's [directives](concepts.md#directives),
such as `"use client"`, are recorded too, and contracts can see through
[re-exports](concepts.md#re-exports).

**Resolution.** Specifiers resolve as TypeScript and Node resolve them, with
each file's nearest `tsconfig.json` and its `paths`, project references and
`extends`; the [options](configuration.md) of `[languages.js]` configure it.
Imports of workspace packages resolve to their source, through their `exports`
maps, whether or not `node_modules` is installed. A bare specifier that doesn't
resolve still names its package, such as `npm:react`, so `npm:` contracts work
in CI without installing dependencies. External packages are named without
their subpath, so `lodash/get` is `npm:lodash`. Node builtins always have the
`node:` prefix, and lose their subpath too: `fs/promises` is `node:fs`.

A `tsconfig.json` that extends one from a package, even a workspace package,
can only be loaded once the package is installed. Until then, its files
resolve without it, so their path aliases don't resolve, and `zenith check`
says so. `ignore_unresolved` silences the specifiers that are expected not to
resolve, such as `virtual:*` modules.

## Go

```toml
[languages.go]
```

**Packages.** The project's modules are found from its `go.mod` and `go.work`
files. Each is named by its module path, so `pkg:example.com/api` selects a
module.

**Imports.** Every import is a `value` import. An import of a package in the
project is an import of its directory, which is one node of the graph, so a
contract names `internal/store` and not the files in it.

**Resolution.** Import paths resolve against the modules that the `go.mod` and
`go.work` files declare. An import of the standard library is a `go:` builtin,
such as `go:net/http`, and keeps its whole path. An import of any other module
is a `gomod:` package, named by the module that provides it, such as
`gomod:github.com/pkg/errors`. A module that no `go.mod` requires is named from
the import path alone.

[`zenith-ignore` comments](suppressions.md) work among a file's imports, as
`//` or `/* */` comments.
