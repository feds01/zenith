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
just lint     # clippy -D warnings + rustfmt --check
just fmt      # format
just run      # run the CLI from source
```

The toolchain is pinned nightly (`rust-toolchain.toml`). This is needed for the
`rustfmt.toml` options `imports_granularity` and `group_imports`. Run
`just lint` before committing; CI runs fmt, clippy, tests on
Linux/macOS/Windows, and `cargo deny`.

## Architecture

Pipeline: config → walk → parse + resolve (parallel) → graph → contracts (parallel) → render.

The crates and what each one owns are listed in
[SPEC §11.2](docs/SPEC.md#112-crates). That table is the single source; update
it when responsibilities move. On `main` today:

- `zenith`: the CLI (`cli.rs`, `commands/*`, `crash.rs`, and `version.rs`
  with `build.rs` for git commit stamping).
- `zenith_graph`: the module tree and import graph model, import kinds, node
  sets, and graph views that leave out imports.
- `zenith_resolve`: specifier resolution with `oxc_resolver`, its
  classification, and `exports`-aware workspace resolution.
- `zenith_utils`: file reading, path and specifier helpers, byte ranges, and
  the glob dialect and name patterns.
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
- Use British spelling in prose (colour, normalise, behaviour).
- Commits are conventional (`feat(graph): …`, `fix(resolve): …`). Branches are
  `feds01/<topic>`.
- **Never commit details of private codebases** (paths, package names). Keep
  real-world corpus configs in the gitignored `corpus/` directory.
