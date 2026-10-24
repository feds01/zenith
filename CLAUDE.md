# Zenith: guide for contributors and agents

Zenith is an architecture linter for monorepos in JavaScript, TypeScript, Go,
Python, Rust and Terraform, written in Rust. The user guide in
[docs/book](docs/book/src/SUMMARY.md) describes its behaviour: when a change
alters what users see, update the guide in the same change.

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
just lint     # clippy -D warnings + rustfmt --check + zenith's own contracts
just fmt      # format
just run      # run the CLI from source
just bench    # benchmarks; extra args go to divan, e.g. `just bench walk`
just bench-ci  # measure the pushed branch with CodSpeed in CI
just bench-instructions <bench> [filter]  # measure with CodSpeed locally, in Docker
just corpus <path> <config>  # a release build on a local checkout, with a config in corpus/
just book     # serve the user guide in docs/book (mdBook)
just changelog <version>  # write CHANGELOG.md for a release, from the titles of the merged PRs
just vscode::test  # the VS Code extension's tests, in VS Code, with this checkout's zenith
```

zenith checks its own architecture: the contracts in the root `zenith.toml`
run in `just lint`, in the pre-commit hook and in CI. A new crate needs a
place in its `crate-layers` contract.

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
- `zenith_config`: the `zenith.toml` types, schema, discovery and validation,
  and the interface through which each language owns its `[languages]` table.
  It knows no language.
- `zenith_contracts`: selectors, the contract types and the violations they
  find, suppressions, the baseline, the JSON output of `zenith check`, and
  the contracts suggested from the import graph.
- `zenith_diagnostics`: reports with source excerpts, the catalogue of
  what zenith says to its users, the `Diagnostic` trait through which an
  error makes its report, and how reports render.
- `zenith_graph`: the module tree and import graph, views of it, and the
  algorithms that run on them.
- `zenith_lang`: the interface that each supported language implements, and
  the types they share: what a file imports, its suppressions and syntax
  errors, what an import resolves to, and a project's workspace packages. It
  depends on no parser or resolver.
- `zenith_lang_go`: Go support: parsing with tree-sitter, and resolving
  import paths against the project's Go modules.
- `zenith_lang_js`: JavaScript and TypeScript support: parsing with oxc,
  specifier resolution with `oxc_resolver`, workspace package discovery
  from `package.json`, and the `[languages.js]` table.
- `zenith_lang_python`: Python support: package discovery from
  `pyproject.toml`, parsing with Ruff's parser, and resolving module paths
  against the roots that absolute imports start from.
- `zenith_lang_rust`: Rust support: crate discovery with Cargo or from the
  manifests, parsing with rust-analyzer's parser, and resolving imports along
  each target's module tree.
- `zenith_lang_terraform`: Terraform and Terragrunt support: parsing their
  module sources and dependency paths with tree-sitter, resolving them to
  directories or registry and repository modules, and the
  `[languages.terraform]` table.
- `zenith_lsp`: the language server of `zenith lsp`: the protocol over
  stdio, and reports as the diagnostics it publishes to an editor. It knows
  nothing of what is checked.
- `zenith_macros`: the macros through which code names the messages of
  the catalogue, `#[derive(Diagnostic)]` and `message!`, which check them
  when it compiles.
- `zenith_pipeline`: the stages that turn a project into its import graph,
  the session they share, and the registry of the languages zenith reads.
- `zenith_reporting`: the output formats of `zenith check`.
- `zenith_utils`: shared helpers: files, paths, globs, locations, terminal
  output and the thread pool.
- `zenith_workspace`: the file walker and git.

Outside the crates, `editors/vscode` is the VS Code extension, in
TypeScript: it finds a zenith binary and runs `zenith lsp` for each
workspace folder that zenith checks.

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
- Keep what zenith says to its users in the catalogue,
  `crates/zenith_diagnostics/messages/en-US/*.ftl`, not in code. Name a
  message with `#[derive(Diagnostic)]` and `#[diag(id)]` on an error type,
  or with `message!(id, ...)`. Ids are snake_case, starting with their
  area, as in `config_unknown_key`.
- Use British spelling in prose (colour, normalise, behaviour).
- Commits are conventional (`feat(graph): …`, `fix(resolve): …`). Branches are
  `feds01/<topic>`.
- **Never commit details of private codebases** (paths, package names). Keep
  real-world corpus configs in the gitignored `corpus/` directory.
