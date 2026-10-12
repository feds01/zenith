# Concepts

## The project root

The project root is the directory with the `zenith.toml`. zenith finds it by
searching upward from the working directory, or takes it from `--config` or
`--root`. Every path in the configuration, in the output and in the baseline
is a POSIX path relative to the root, with `/` separators on every platform.

## Nodes

zenith models the codebase as a tree of nodes:

| Node | Example | |
|---|---|---|
| Directory | `web/src/domains` | Every ancestor directory of a file. A directory stands for its whole subtree. |
| File | `web/src/domains/ai/api/client.ts` | Every source file, and every file inside the root that an import resolves to, such as `.css` or `.json`. |
| External package | `npm:react`, `npm:@tanstack/react-query` | Named by the package, so `lodash/get` is `npm:lodash`. |
| Builtin | `node:fs`, `bun:sqlite` | Node builtins always have the `node:` prefix, and subpaths are dropped: `fs/promises` is `node:fs`. |

An `index.ts` is an ordinary file, with no special role for its directory.

## Workspace packages

Workspace packages are found from the root `package.json`'s `workspaces`
field and from `pnpm-workspace.yaml`. Each has a name, from its own
`package.json`, and a directory. Names and directories often differ, so
contracts can name packages with [`pkg:` selectors](selectors.md).

## Imports

An import is an edge from a file to a file, package or builtin. Each import
statement is recorded with its kind:

| Kind | Made by |
|---|---|
| `value` | `import x from "s"`, `import "s"`, `export … from "s"` |
| `type` | `import type …`, `export type …`, imports whose every name is `type`, and TypeScript import types such as `typeof import("s")` |
| `dynamic` | `import("s")` with a literal specifier |
| `require` | `require("s")` with a literal specifier, and `import x = require("s")` |

Imports whose specifier isn't a literal, such as `import(path)`, are skipped
and counted; `-v` lists them. Contracts can leave out imports by kind with
[`exclude_kinds`](contracts/overview.md#shared-options): type-only imports
are erased at runtime, so they are often excluded.

## Units

Contracts reason about **units**: labelled sets of nodes, which
[selectors](selectors.md) pick. A matched directory is one unit, with its
whole subtree as members; a matched file, package or builtin is a unit of one.
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
