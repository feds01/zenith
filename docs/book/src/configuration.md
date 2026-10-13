# Configuration

`zenith.toml` lives in the project root. Unknown keys are errors, with
suggestions for likely typos. Editors can complete and check the file from its
[JSON Schema](https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json),
which `zenith init` points to with a `#:schema` comment.

```toml
#:schema https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json

[files]
include = ["**/*.{ts,tsx,mts,cts,js,jsx,mjs,cjs}"]   # the default
exclude = ["experimental/**", "**/*.gen.ts"]       # added to the defaults below
respect_gitignore = true                           # the default

[resolve]
tsconfig = "auto"          # the nearest tsconfig.json of each file; "off"; or a path
conditions = ["types", "source", "import", "module", "require", "node", "default"]
main_fields = ["module", "main"]
alias = { "types" = "web/src/types" }              # aliases only a bundler knows
ignore_unresolved = ["/*", "virtual:*"]            # specifiers not to warn about

[analysis]
exclude_kinds = []         # the import kinds that every contract leaves out
barrels = "opaque"         # or "transparent": how contracts see barrels

[baseline]
path = "zenith.baseline.toml"
stale = "error"            # or "warn"

[contracts.<id>]
# see the contract types
```

## Files

`node_modules`, `.git`, `.yarn`, `coverage`, `.next`, `.nuxt`, `.output`,
`.svelte-kit`, `.turbo`, `.vercel`, `.cache`, `.wxt`, `.mastra` and
`storybook-static` are always excluded, wherever they are, and so are Yarn's
`.pnp.cjs` and `.pnp.loader.mjs`. `dist`, `build` and `out` are excluded only
next to a `package.json`, since elsewhere, as in `src/build/`, they may hold
source code.

## Resolution

Specifiers resolve as TypeScript and Node resolve them, with each file's
nearest `tsconfig.json` and its `paths`, project references and `extends`.
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
`[resolve] ignore_unresolved` silences the expected ones. To see how one
specifier resolves, and why, run `zenith resolve <file> <specifier>`.
