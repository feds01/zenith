# Zenith: guide for contributors and agents

Zenith is an architecture linter for TypeScript/JavaScript monorepos, written
in Rust. The user guide in [docs/book](docs/book/src/SUMMARY.md) describes its
behaviour: when a change alters what users see, update the guide in the same
change.

## How the work lands

`main` is built up one reviewed PR at a time. A complete prototype of milestones M0–M4
lives on the `feds01/full-implementation` branch. Each PR ports one piece of
it, with its tests, and may refine it in review. Keep every PR building, tested
and lint-clean on its own.

## Commands

```sh
just test     # all tests; extra args go to cargo, e.g. `just test -p zenith`
just test-ui  # the UI snapshot tests in tests/cases, e.g. `just test-ui acme`
just update-snapshots  # re-record their output.snap files
just lint     # clippy -D warnings + rustfmt --check
just fmt      # format
just run      # run the CLI from source
just bench    # benchmarks; extra args go to divan, e.g. `just bench walk`
just bench-ci  # measure the pushed branch with CodSpeed in CI
just bench-instructions <bench> [filter]  # measure with CodSpeed locally, in Docker
just corpus <path> <config>  # a release build on a local checkout, with a config in corpus/
just book     # serve the user guide in docs/book (mdBook)
```

The toolchain is pinned nightly (`rust-toolchain.toml`). This is needed for the
`rustfmt.toml` options `imports_granularity` and `group_imports`. Run
`just lint` before committing; CI runs fmt, clippy, tests on
Linux/macOS/Windows, and `cargo deny`.

## Architecture

Pipeline: config → discover → parse (parallel) → resolve (parallel) → graph → contracts (parallel) → render.

The crates, and what each one owns. Update a line only when a crate takes on
a new kind of responsibility, not for each feature:

- `zenith`: the CLI, its commands, and the UI snapshot harness in
  `tests/ui.rs`.
- `zenith_benchmark`: the benchmarks, and the synthetic monorepo they run on.
- `zenith_cache`: the cache of what zenith found in each file between runs,
  and its file format.
- `zenith_config`: the `zenith.toml` types, schema, discovery and validation.
- `zenith_contracts`: selectors, the contract types and the violations they
  find, suppressions, the baseline, and the JSON output of `zenith check`.
- `zenith_diagnostics`: reports with source excerpts, and how they render.
- `zenith_graph`: the module tree and import graph, views of it, and the
  algorithms that run on them.
- `zenith_lang`: the language-neutral types that every language shares, such
  as what an import resolves to and a project's workspace packages. It
  depends on no parser or resolver.
- `zenith_lang_js`: JavaScript and TypeScript support: specifier resolution
  with `oxc_resolver` and workspace package discovery from `package.json`.
- `zenith_parse`: what each source file imports and exports, read with oxc.
- `zenith_pipeline`: the stages that turn a project into its import graph,
  and the session they share.
- `zenith_reporting`: the output formats of `zenith check`.
- `zenith_utils`: shared helpers: files, paths, globs, locations, terminal
  output and the thread pool.
- `zenith_workspace`: the file walker and git.

Key invariants:

- **Node ids** are POSIX paths relative to the canonical root, or
  `npm:<pkg>` / `node:<builtin>`.
- **The graph is never mutated.** Each contract checks against its own view.
- **Output must be deterministic.** Sort before rendering.
- **All log output goes to stderr**; reports go to stdout.
- **Exit codes** are in [the guide](docs/book/src/cli.md#exit-codes): 0 kept,
  1 broken, 2 configuration, usage or internal error.

## Conventions

- Prefer bracketlint (`~/code/feds01/bracketlint`) as the reference for
  conventions and utilities, with teamsearch as a fallback.
- Every file starts with a `//!` module doc. Public items and struct fields
  get `///` docs.
- Number the key steps of a command, or of another long function, with
  comments: `// 1. Open the project, and build its graph.`, `// 2. …`.
- Keep commands thin: logic that isn't about the command line belongs in the
  core crates, with the command only gluing it together.
- Use British spelling in prose (colour, normalise, behaviour).
- Commits are conventional (`feat(graph): …`, `fix(resolve): …`). Branches are
  `feds01/<topic>`.
- **Never commit details of private codebases** (paths, package names). Keep
  real-world corpus configs in the gitignored `corpus/` directory.
