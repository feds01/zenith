# Configuration

`zenith.toml` lives in the project root. Unknown keys are errors, with
suggestions for likely typos. Editors can complete and check the file from its
[JSON Schema](https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json),
which `zenith init` points to with a `#:schema` comment.

```toml
#:schema https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json

[files]
exclude = ["experimental/**", "**/*.gen.ts"]       # added to the defaults below
respect_gitignore = true                           # the default

[languages.js]                                     # also accepted as [languages.typescript]
include = ["**/*.{ts,tsx,mts,cts,js,jsx,mjs,cjs}"]   # the default
tsconfig = "auto"          # the nearest tsconfig.json of each file; "off"; or a path
conditions = ["types", "source", "import", "module", "require", "node", "default"]
main_fields = ["module", "main"]
alias = { "types" = "web/src/types" }              # aliases only a bundler knows
ignore_unresolved = ["/*", "virtual:*"]            # specifiers not to warn about

[languages.go]                                     # off unless the table is written
include = ["**/*.go"]                              # the default

[analysis]
exclude_kinds = []         # the import kinds that every contract leaves out
reexports = "opaque"         # or "transparent": how contracts see pass-through modules

[baseline]
path = "zenith.baseline.toml"
stale = "error"            # or "warn"

[cache]
enabled = true             # the default; `--no-cache` turns it off for one run
dir = ".zenith_cache"      # the default, relative to the root

[contracts.<id>]
# see the contract types
```

## Languages

Each language zenith reads has a table of its own in `[languages]`, and a
language is enabled by writing its table. The table may be empty:
`[languages.js]` enables JavaScript and TypeScript with their defaults, and
`[languages.go]` enables Go. Without a `[languages]` table, JavaScript and
TypeScript are enabled. A `[languages]` table with no language in it is an
error, since there would be nothing to scan.

`js` is the name of the JavaScript and TypeScript table, and `typescript` is
accepted for it too. Giving both is an error.

A language owns its options, and the selector schemes that only its projects
have: `npm:`, `node:` and `bun:` belong to `js`, and `go:` and `gomod:` belong
to `go`, so a selector that uses one is an error when its language is not
enabled. `pkg:` selects workspace packages and
belongs to no language.

Each language scans the files that its `include` globs match, which default to
its own source files: every `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`,
`.mjs` and `.cjs` file for `js`, and every `.go` file for `go`. The `[files]` table is about what is skipped,
whatever the language.

A path selector may leave the extension off a file, so `src/types` matches the
file `src/types.ts` as well as the directory `src/types`. The extensions it
may leave off are those of the enabled languages.

## Files

`node_modules`, `.git`, `.yarn`, `coverage`, `.next`, `.nuxt`, `.output`,
`.svelte-kit`, `.turbo`, `.vercel`, `.cache`, `.wxt`, `.mastra` and
`storybook-static` are always excluded, wherever they are, and so are Yarn's
`.pnp.cjs` and `.pnp.loader.mjs`. `dist`, `build` and `out` are excluded only
next to a `package.json`, since elsewhere, as in `src/build/`, they may hold
source code.

## Resolution

JavaScript and TypeScript specifiers resolve as TypeScript and Node resolve
them, with each file's nearest `tsconfig.json` and its `paths`, project
references and `extends`; `[languages.js]` configures it.
Imports of workspace packages resolve to their source, through their
`exports` maps, whether or not `node_modules` is installed. A bare specifier
that doesn't resolve still names its package, such as `npm:react`, so
`npm:` contracts work in CI without installing dependencies.

A `tsconfig.json` that extends one from a package, even a workspace package,
can only be loaded once the package is installed. Until then, its files
resolve without it, so their path aliases don't resolve, and `zenith check`
says so.

Imports that can't be resolved are left out of the graph, and `zenith check`
warns about them: `--show-unresolved` lists them, and
`[languages.js] ignore_unresolved` silences the expected ones. To see how one
specifier resolves, and why, run `zenith resolve <file> <specifier>`.

Go import paths resolve against the modules that the project's `go.mod` and
`go.work` files declare. Each module is a workspace package named by its module
path, so `pkg:example.com/api` selects a module. An import of a package in the
project is an import of its directory, which is one node of the graph, so a
contract names `internal/store` and not the files in it. An import of the
standard library is a `go:` builtin, such as `go:net/http`, and one of any other
module is a `gomod:` package, named by the module that provides it, such as
`gomod:github.com/pkg/errors`. A module that no `go.mod` requires is named from
the import path alone.

## Cache

zenith keeps what it finds in each file in a cache, so that a run after an
edit neither reads nor parses the files that haven't changed. A file is
taken from the cache without being read if its size, times and inode are
unchanged, or without being parsed if its contents are. The cache never
changes what a check finds.

The cache is one file in `[cache] dir`, `.zenith_cache` by default. zenith creates the directory, with a
`.gitignore` that ignores everything in it, so it is never committed. A new
version of zenith, or a configuration that needs more from each file, such as
`reexports = "transparent"`, starts a new cache; a cache that is corrupt, or
can't be written, is skipped.
