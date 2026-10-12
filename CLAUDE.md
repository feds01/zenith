# Zenith: guide for contributors and agents

Zenith is an architecture linter for TypeScript/JavaScript monorepos, written
in Rust. **[docs/SPEC.md](docs/SPEC.md) is the source of truth for behaviour.**
When implementation and spec disagree, fix one of them in the same change.

## How the work lands

`main` is built up one reviewed PR at a time, in the order of the spec's
architecture (§11) and roadmap (§13). A complete prototype of milestones M0–M4
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
just corpus <path> <config>  # a release build on a local checkout, with a config in corpus/
```

The toolchain is pinned nightly (`rust-toolchain.toml`). This is needed for the
`rustfmt.toml` options `imports_granularity` and `group_imports`. Run
`just lint` before committing; CI runs fmt, clippy, tests on
Linux/macOS/Windows, and `cargo deny`.

## Architecture

Pipeline: config → discover → parse (parallel) → resolve (parallel) → graph → contracts (parallel) → render.

The crates and what each one owns are listed in
[SPEC §11.2](docs/SPEC.md#112-crates). That table is the single source; update
it when responsibilities move. On `main` today:

- `zenith`: the CLI (`cli.rs`, `commands/*`, `crash.rs`, `logger.rs`, and
  `version.rs` with `build.rs` for git commit stamping), `project.rs`, which
  finds the project and its configuration and runs the pipeline on it, and
  the UI snapshot harness in `tests/ui.rs`.
- `zenith_benchmark`: the divan benchmarks, and the deterministic synthetic
  monorepo that they run on.
- `zenith_config`: the `zenith.toml` types and JSON Schema, config discovery,
  parsing, and validation into typed contract definitions, including the
  syntax of selectors, layers and ignored imports, and the starter
  configuration that `zenith init` writes.
- `zenith_contracts`: resolving selectors into units, the labelled sets of
  nodes that contracts reason about, from what the graph and the workspace
  find, and failing on path and `pkg:` selectors that match nothing;
  compiling contracts against a project, the view of the graph that each
  checks against (without its excluded kinds, ignored imports, suppressed
  statements and baselined imports), the six contract types, the
  violations they find, the suppressions that had no effect, the baseline
  file of known violations, their reports and summary entries, and the JSON
  output of `zenith check`, with the pipeline's problems in it.
- `zenith_diagnostics`: reports with labelled source excerpts, which each
  crate makes from its own errors and findings, rendered in full or a line
  each using `annotate-snippets`, or as GitHub Actions workflow commands;
  the `SourceMap`, which reads a project's files when reports first need
  them and finds where a span is; and errors and warnings collected per
  pass.
- `zenith_graph`: the module tree and import graph model, import kinds, node
  sets, units, finding the nodes that a path glob or name pattern picks out,
  graph views that leave out imports (by pair or by kind), the grimp
  algorithms that run on them: shortest import chains, layers and cycle
  breakers, and module overviews for visualisations (`zenith graph`).
- `zenith_parse`: oxc-based extraction of imports and their kinds,
  directives and suppression comments.
- `zenith_pipeline`: the stages (discover, parse, resolve, graph) that turn
  a project into its import graph, registered in order on a `Pipeline` that
  runs and times them, and the `Session` they share. Each stage is a thin
  adapter over the crate that does its work. Files it can't read or parse
  are recorded as `FileError`s, and imports it can't resolve as
  `UnresolvedImport`s, with reports and JSON.
- `zenith_reporting`: the output of `zenith check`, without knowing what
  is checked: the reports that the domain crates make, then the summary of
  every contract, in the `pretty`, `compact` and `github` formats.
- `zenith_resolve`: specifier resolution with `oxc_resolver`, its
  classification, and `exports`-aware workspace resolution.
- `zenith_utils`: file reading, path and specifier helpers, byte ranges, the
  glob dialect and name patterns, the line index, terminal output:
  highlighting, the logger, and printing text and trees, and the global
  thread pool.
- `zenith_workspace`: the file walker and workspace package discovery.

Key invariants:

- **Node ids** are POSIX paths relative to the canonical root, or
  `npm:<pkg>` / `node:<builtin>`.
- **The graph is never mutated.** Each contract checks against its own view.
- **Output must be deterministic.** Sort before rendering.
- **All log output goes to stderr**; reports go to stdout.
- **Exit codes** are in [SPEC §9](docs/SPEC.md#9-cli): 0 kept, 1 broken, 2
  configuration, usage or internal error.

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
