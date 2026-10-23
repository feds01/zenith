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

[languages.js]             # a table per language, with the options on its page

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

`[languages]` has a table for each language to read, which enables it, with
the options that the language's page lists. Without a `[languages]` table,
JavaScript and TypeScript are enabled. See [languages](languages.md).

## Files

`node_modules`, `.git`, `.yarn`, `coverage`, `.next`, `.nuxt`, `.output`,
`.svelte-kit`, `.turbo`, `.vercel`, `.cache`, `.wxt`, `.mastra` and
`storybook-static` are always excluded, wherever they are, and so are Python's
`.venv`, `site-packages`, `__pycache__`, `.tox`, `.nox`, `.mypy_cache`,
`.pytest_cache` and `.ruff_cache`, and Yarn's `.pnp.cjs` and
`.pnp.loader.mjs`. `dist`, `build` and `out` are excluded only
next to a `package.json`, since elsewhere, as in `src/build/`, they may hold
source code.

## Resolution

Each language resolves its imports its own way, which its
[page](languages.md) describes. Imports that can't be resolved are left out of
the graph, and `zenith check` warns about them: `--show-unresolved` lists them.
To see how one import resolves, and why, run
`zenith resolve <file> <specifier>`.

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
