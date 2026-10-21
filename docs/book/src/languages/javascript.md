# JavaScript and TypeScript

```toml
[languages.js]                                     # also accepted as [languages.typescript]
include = ["**/*.{ts,tsx,mts,cts,js,jsx,mjs,cjs}"]   # the default
tsconfig = "auto"          # the nearest tsconfig.json of each file; "off"; or a path
conditions = ["types", "source", "import", "module", "require", "node", "default"]
main_fields = ["module", "main"]
extensions = [".ts", ".tsx", ".d.ts", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"]
alias = { "types" = "web/src/types" }              # aliases only a bundler knows
ignore_unresolved = ["/*", "virtual:*"]            # specifiers not to warn about
```

JavaScript and TypeScript are enabled when the configuration has no
`[languages]` table. The table is called `js`, and `typescript` is accepted for
it too; giving both is an error.

| Option | |
|---|---|
| `include` | Globs of the files to scan. |
| `tsconfig` | Which `tsconfig.json` files to use: `"auto"` uses the nearest one of each file, `"off"` uses none, and any other value is the path of the one to use for every file. |
| `conditions` | The condition names to match in `exports` maps, replacing the defaults. |
| `main_fields` | The `package.json` fields that point at a package's entry point, replacing the defaults. |
| `extensions` | The extensions to try, in order, when a specifier has none, replacing the defaults. |
| `alias` | Aliases that only a bundler knows: a specifier prefix, and the path that it maps to. |
| `ignore_unresolved` | Name patterns of the specifiers that are expected not to resolve, so are not reported as unresolved. |

## Packages

Workspace packages are found from the root `package.json`'s `workspaces`
field and from `pnpm-workspace.yaml`. Each has a name, from its own
`package.json`, and a directory, so `pkg:@acme/ui` selects the package named
`@acme/ui` wherever it lives.

## Imports

Each import statement is recorded with its kind:

| Kind | Made by |
|---|---|
| `value` | `import x from "s"`, `import "s"`, `export … from "s"` |
| `type` | `import type …`, `export type …`, imports whose every name is `type`, and TypeScript import types such as `typeof import("s")` |
| `dynamic` | `import("s")` with a literal specifier |
| `require` | `require("s")` with a literal specifier, and `import x = require("s")` |

Imports whose specifier isn't a literal, such as `import(path)`, are skipped
and counted; `-v` lists them. A file's [directives](../concepts.md#directives),
such as `"use client"`, are recorded too, and contracts can see through
[re-exports](../concepts.md#re-exports).

## Resolution

Specifiers resolve as TypeScript and Node resolve them, with each file's
nearest `tsconfig.json` and its `paths`, project references and `extends`.
Imports of workspace packages resolve to their source, through their `exports`
maps, whether or not `node_modules` is installed.

| Imported | Node | |
|---|---|---|
| a file of the project | `web/src/lib/format.ts` | |
| an external package | `npm:react` | Named without its subpath, so `lodash/get` is `npm:lodash`. A bare specifier that doesn't resolve still names its package, so `npm:` contracts work in CI without installing dependencies. |
| a builtin | `node:fs`, `bun:sqlite` | Node builtins always have the `node:` prefix, and lose their subpath too: `fs/promises` is `node:fs`. |

A `tsconfig.json` that extends one from a package, even a workspace package,
can only be loaded once the package is installed. Until then, its files
resolve without it, so their path aliases don't resolve, and `zenith check`
says so.
