# Concepts

## The project root

The project root is the directory with the `zenith.toml`. zenith finds it by
searching upward from the working directory, or takes it from `--config` or
`--root`. Every path in the configuration, in the output and in the baseline
is a POSIX path relative to the root, with `/` separators on every platform.

## Nodes

zenith models the codebase as a tree of nodes, with the external packages and
builtins that it imports:

<figure class="diagram">
{{#include diagrams/nodes.svg}}
<figcaption>A node's id is its path from the project root. Packages and builtins are named without their subpath.</figcaption>
</figure>

| Node | Example | |
|---|---|---|
| Directory | `web/src/domains` | Every ancestor directory of a file. A directory stands for its whole subtree, and is also what an import of a Go package refers to. |
| File | `web/src/domains/ai/api/client.ts` | Every source file, and every file inside the root that an import resolves to, such as `.css` or `.json`. |
| External package | `npm:react`, `gomod:github.com/pkg/errors` | A package from outside the project, with its language's scheme. |
| Builtin | `node:fs`, `go:net/http` | A module of a runtime or standard library, with its language's scheme. |

Each [language](languages.md) says how it names its external packages and
builtins. A file such as an `index.ts` is an ordinary file, with no special
role for its directory.

## Workspace packages

Each language finds the packages of the project: the workspace packages of a
JavaScript monorepo, and the modules of a Go project. Each has a name and a
directory, and names and directories often differ, so contracts can name
packages with [`pkg:` selectors](selectors.md). The [languages](languages.md)
page says where each language finds them.

<figure class="diagram">
{{#include diagrams/packages.svg}}
<figcaption>A <code>pkg:</code> selector matches packages by name, and picks their directories.</figcaption>
</figure>

## Imports

An import is an edge from a file to a file, directory, package or builtin.
Each import statement is recorded with its kind:

<figure class="diagram">
{{#include diagrams/imports.svg}}
<figcaption>Each statement is an edge with its kind. A specifier that isn't a literal is skipped.</figcaption>
</figure>

| Kind | |
|---|---|
| `value` | an ordinary import, which every language has |
| `type` | an import that is erased at runtime, such as TypeScript's `import type` |
| `dynamic` | an import made while the code runs, such as JavaScript's `import("s")` |
| `require` | a CommonJS `require("s")` |

The [languages](languages.md) page lists the statements that make each kind.
Contracts can leave out imports by kind with
[`exclude_kinds`](contracts/overview.md#common-options): type-only imports
are erased at runtime, so they are often excluded.

## Re-exports

In JavaScript and TypeScript, a **pass-through module** only re-exports other
modules, such as an `index.ts` of `export * from "./a"` and
`export { b } from "./b"` lines, with no code of its own. JavaScript calls it a
barrel. Through it, the import graph has a chain to every module it
re-exports, so `import { format } from "./lib"` reaches whatever else
`lib/index.ts` re-exports, even server code. Bundlers that know the module has
no side effects leave the unused modules out.

A contract with [`reexports = "transparent"`](contracts/overview.md#common-options)
sees an import of names from a pass-through module as imports of the modules
that define them, following re-exports and `export *` through nested
pass-through modules. An import that takes the whole module, such as
`import * as lib from "./lib"`, still imports all of it, and so does an
import of a name that can't be followed, such as one that may come from an
external package through `export *`. Importing only a name's type is a
`type` import.

```toml
[contracts.client-safety]
type = "forbidden"
from = [{ path = "web/src", directive = "use client" }]
to = ["node:*", "npm:sharp"]
reexports = "transparent"
exclude_kinds = ["type"]
```

`encapsulation` contracts always see re-exports as they are, since importing
a module through its entry point, which often only re-exports, is what they
allow.

## Units

Contracts reason about **units**: labelled sets of nodes, which
[selectors](selectors.md) pick. A matched directory is one unit, with its
whole subtree as members; a matched file, package or builtin is a unit of one.

<figure class="diagram">
{{#include diagrams/units.svg}}
<figcaption>Each directory that <code>web/src/features/*</code> matches is a unit, with everything under it.</figcaption>
</figure>

Two units **overlap** if they share a member, and contracts skip pairs of
overlapping units, so `from = ["a"]` and `to = ["a/*"]` checks nothing between
`a` and itself.

## Directives

A directive is a string literal at the very top of a file, before any other
code, such as `"use client"`. In React Server Components, `"use client"` makes
a file the entry point of a client bundle, so everything it imports ships to
the browser. zenith records every directive as written, and a
[`directive` selector](selectors.md#the-table-form) can pick the files that
have one, to draw a boundary that the directory layout doesn't show.

<figure class="diagram">
{{#include diagrams/directives.svg}}
<figcaption>The client components start the client bundle, wherever they are. A <code>directive</code> selector makes each of them a unit.</figcaption>
</figure>
