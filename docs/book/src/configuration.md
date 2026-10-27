# Configuration

`zenith.toml` lives in the project root, and may be [split over several
files](#splitting-the-configuration). Unknown keys are errors, with
suggestions for likely typos. Editors can complete and check the file from its
[JSON Schema](https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json),
which `zenith init` points to with a `#:schema` comment.

```toml
#:schema https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json

include = ["zenith/*.toml"]  # more files of contracts, as if written here

[workspace]
members = ["apps/*"]       # directories with a `zenith.toml` of their own

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

## Splitting the configuration

A large project's configuration can be split over several files, which
zenith reads as one: `zenith check` in the root checks the contracts of them
all, against one import graph.

`include` lists globs of files whose contracts are read as if they were
written in the file that includes them, with the same paths and ids. It suits
a long list of contracts, split by topic. A file that is included may include
others in turn, and each file is read once.

`[workspace] members` lists globs of directories, such as the apps and
packages of a monorepo, that have a `zenith.toml` of their own, for the
contracts about their own code. A directory without one is skipped. A
member's file:

- has paths relative to its directory, so `src/lib` in `apps/web/zenith.toml`
  is `apps/web/src/lib`. Package and external selectors, such as `pkg:` and
  `npm:`, are the same everywhere;
- names its contracts with ids that start with its directory, a segment at a
  time, so `[contracts.layers]` in `apps/web/zenith.toml` is the contract
  `apps.web.layers` in reports, `--contract` and the baseline. In the files
  of `apps/web`, a `zenith-ignore` comment may name it `layers`;
- adds to the root's contracts, which still apply to its code;
- may include files of its own, with globs relative to its directory, which
  are part of the member: their paths are relative to its directory too,
  their ids start with it, and their `[analysis]` defaults to the member's.

```toml
# apps/web/zenith.toml
[analysis]
exclude_kinds = ["type"]   # for this file's contracts; the root's otherwise

[files]
exclude = ["generated/**"] # added to the root's

[contracts.layers]
type = "layers"
layers = ["src/app", "src/features", "src/lib"]
```

A file that the root includes, or a member's, may have `include`,
`[contracts]`, `[analysis]`, whose options apply to its own contracts and
default to the root's, and `[files] exclude`, which adds to the root's. The
other tables and options are the root's alone. Contract ids must be unique
across the files, and each file is part of one configuration only: a member's
own `zenith.toml` can't be included, and two members, or the root and a
member, can't include the same file.

The globs of `include` and `members` can't have a `..` segment, and may have
`**`, as in `services/**`. They are matched against the files that zenith
would walk, so they skip the directories that zenith never scans, the root's
`[files] exclude` and, unless `respect_gitignore = false`, what `.gitignore`
ignores. The globs of a file's `include` also skip what its own `[files]
exclude`, and that of each file that includes it, excludes. A glob that
matches nothing is an error.

zenith finds the root's file from a member's directory too, so `zenith check`
in `apps/web` checks the whole project. To report only on a member's code,
give its directory, as in `zenith check apps/web`, or check its contracts
alone, as in `zenith check --contract apps.web.layers`.

## Languages

`[languages]` has a table for each language to read, which enables it, with
the options that the language's page lists. Without a `[languages]` table,
JavaScript and TypeScript are enabled. See [languages](languages.md).

## Files

`node_modules`, `.git`, `.yarn`, `coverage`, `.next`, `.nuxt`, `.output`,
`.svelte-kit`, `.turbo`, `.vercel`, `.cache`, `.wxt`, `.mastra` and
`storybook-static` are always excluded, wherever they are, and so are Python's
`.venv`, `site-packages`, `__pycache__`, `.tox`, `.nox`, `.mypy_cache`,
`.pytest_cache` and `.ruff_cache`, Terraform's `.terraform` and
`.terragrunt-cache`, and Yarn's `.pnp.cjs` and `.pnp.loader.mjs`. `dist`,
`build` and `out` are excluded only next to a `package.json`, since elsewhere,
as in `src/build/`, they may hold source code.

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
