# zenith_benchmark

Benchmarks for the zenith crates, and the synthetic monorepo that they run
on.

## The synthetic repository

`SyntheticRepo::new(n)` generates a TypeScript monorepo with exactly `n`
source files, in memory. The generator uses a fixed-seed PRNG, so a size
always gives the same files, byte for byte.

It has about one `@acme/...` package per 250 files: libraries in `packages/`
(some with `exports` maps, and with camel-cased directories), Node.js
services in `services/` and Next.js apps in `apps/`, each with a
`tsconfig.json` that adds an `@/*` alias. Their `src/features/` directories,
some of them nested, hold components, hooks, API clients, utilities,
handlers, jobs, stores and tests behind barrel `index.ts` files. Files import
each other about five times each, through relative paths, the alias,
workspace packages (by name, `exports` subpath or deep path), npm packages
and `node:` builtins, with some type-only, dynamic and `require` imports.
Each import records the file that it resolves to, if it's in the repository.
The repository also has what the walk must skip: stubs in `node_modules`,
packages' `dist` output, `.next` builds and `.gitignore`d `__generated__`
code.

`cargo test -p zenith_benchmark` checks that generation is deterministic,
that each size has exactly its number of source files, and that every
specifier leads to the file that its import records.
