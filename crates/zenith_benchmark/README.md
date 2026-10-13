# zenith_benchmark

Benchmarks for the zenith crates, written with
[divan](https://github.com/nvzqz/divan) through CodSpeed's
`codspeed-divan-compat`.

## Running

```sh
just bench               # every benchmark
just bench walk          # only those whose name matches a regex
just bench glob_is_match --sample-count 20
```

Extra arguments go to divan, so `just bench --help` lists its options.
The benchmarks allocate with mimalloc, as the `zenith` binary does.
Allocations are counted with divan's `AllocProfiler`, which only sees the
benchmark's own thread, so the allocations of the walk's worker threads
don't show up in its counts, and nor do the parse and resolve workers'.

In CI, the CodSpeed workflow runs them under CodSpeed's simulation and
memory instruments, on every commit to `main` and on PRs that touch the
benchmarked crates. It runs them on one thread, with `RAYON_NUM_THREADS=1`,
which the walk follows too: the simulation counts every thread's
instructions, so a pool's workers spinning and stealing would make the
counts differ from run to run. CodSpeed's numbers are therefore the work
done, not the time taken on many cores; run `just bench` for those.

### Measuring with CodSpeed before opening a PR

CI's CodSpeed numbers are the ones to trust. `just bench-ci` runs the
CodSpeed workflow on the current branch, once it is pushed, so a change can
be measured without a PR, and compared with `main` in CodSpeed's dashboard.

`just bench-instructions <bench> [filter]`, e.g.
`just bench-instructions contracts layers`, measures one bench locally with
CodSpeed's own CLI, under its simulation, in a Linux container
(`codspeed/Dockerfile`), since CodSpeed's valgrind only runs on Linux. It
needs Docker, and a CodSpeed login, once, with `just codspeed-login` (or a
token in `CODSPEED_TOKEN`); each run is uploaded to CodSpeed, which links to
its report. To compare two runs, set `CODSPEED_BASE` to the first run's id.
These counts compare with each other, but not with CI's: the container runs
on the host's architecture (arm64 on Apple silicon), and the build skips fat
LTO to fit in Docker's memory.

## What is measured

| Bench | Benchmark | Input |
|---|---|---|
| `glob` | `glob_new`: compiling a glob | Seven globs, from a literal path to two `**` segments |
| `glob` | `glob_is_match`: matching a glob | The same globs, against every source path of a 10k-file repository |
| `glob` | `name_pattern_is_match`: matching a name pattern | Five patterns, against every workspace and npm package name |
| `workspace` | `walk`: finding the files to scan, with the default settings | Repositories of 1k, 10k and 50k source files |
| `workspace` | `discover_packages`: finding the workspace packages | The same repositories |
| `pipeline` | `pipeline`: running every stage up to the graph, with the default configuration | The same repositories |
| `pipeline` | `parse`, `resolve`, `graph`: each of those stages on its own | The same repositories |
| `pipeline` | `parse_cold_cache`: the parse stage with an empty parse cache, which it writes, as on the first run | The same repositories |
| `pipeline` | `parse_warm_cache`, `pipeline_warm_cache`: the parse stage, and every stage up to the graph, with a parse cache that has every file, as on a run after nothing changed | The same repositories |
| `contracts` | `forbidden_without_chains`, `forbidden_with_chains`, `protected`, `independence`, `layers`, `acyclic`, `encapsulation`: checking a contract, compiled beforehand | The same repositories |
| `baseline` | `record`: finding the baseline entries that keep every contract of `contracts.toml`, starting from an empty baseline, as `zenith baseline` does | The same repositories |
| `check` | `check`: running the pipeline up to the graph, then compiling and checking every contract of `contracts.toml`, as `zenith check` does before showing its results | The same repositories |
| `check` | `contracts`: compiling and checking every contract, in parallel, on a graph built beforehand | The same repositories |

The 50k-file repository is only used locally; CodSpeed's simulation would
take too long on it.

## The synthetic repository

`SyntheticRepo::new(n)` generates a TypeScript monorepo with exactly `n`
source files, in memory; `TempRepo` writes one to a temporary directory, and
`TempRepo::shared` writes each size once per benchmark binary, until the
`TempRepo::remove_shared_on_drop` guard in `main` removes them. The generator
uses a fixed-seed PRNG, so a size always gives the same files, byte for byte.

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
that each size has exactly its number of source files, that every specifier
leads to the file that its import records, and that the walk and package
discovery find what the generator wrote.
