# Zenith: guide for contributors and agents

Zenith is an architecture linter for TypeScript/JavaScript monorepos, written
in Rust. **[docs/SPEC.md](docs/SPEC.md) is the source of truth for behaviour.**
When implementation and spec disagree, fix one of them in the same change.

## Commands

```sh
just test                 # cargo test --workspace (unit + UI snapshot tests)
just test-ui <filter>     # UI snapshot tests only, e.g. `just test-ui features/layers`
just update-snapshots     # re-record tests/cases/**/output.snap (REGENERATE_OUTPUT=1)
just lint                 # clippy -D warnings + rustfmt --check
just fmt                  # format
just run check            # run the CLI from source in the current directory
just schema               # regenerate zenith.schema.json after changing config types
```

The toolchain is pinned nightly (`rust-toolchain.toml`). This is needed for the
`rustfmt.toml` options `imports_granularity` and `group_imports`. Run
`just lint` before committing; CI runs fmt, clippy, tests on
Linux/macOS/Windows, and `cargo deny`.

## Architecture

Pipeline: config → walk → parse + resolve (parallel) → graph → contracts (parallel) → render.

| Crate | Role |
|---|---|
| `zenith` | The CLI: `cli.rs` (clap), `commands/*`, `project.rs` (locating the root and config, index settings), `crash.rs`, `version.rs` and `build.rs`. |
| `zenith_config` | `zenith.toml` types (serde + schemars) and parsing. Selectors stay as strings here. |
| `zenith_workspace` | Parallel file walker (gitignore, default excludes) and workspace package discovery. |
| `zenith_parse` | oxc AST walk that extracts imports (with kinds), directives and `zenith-ignore` comments. |
| `zenith_resolve` | `oxc_resolver` setup; classifies each resolution as internal, `npm:`, builtin or unresolved. Its `exports.rs` resolves workspace packages without `node_modules`. |
| `zenith_graph` | The immutable module tree and import graph, plus grimp ports: shortest chains, layers, cycle breakers. No IO. |
| `zenith_index` | Orchestrates walk, parse and resolve into a `Graph`, deterministically. |
| `zenith_contracts` | Selectors and units, per-contract views, the six contract types, findings, baseline and suppressions. |
| `zenith_reporting` | The `pretty` (annotate-snippets), `compact`, `json` and `github` renderers. |
| `zenith_utils` | Logging, streams, highlighting, glob dialect, printing, tree writing, line index and paths (mostly from bracketlint). |

Key invariants:

- **Node ids** are POSIX paths relative to the canonical root, or
  `npm:<pkg>` / `node:<builtin>`. `ModuleId`s are assigned alphabetically, so
  iterating by id is deterministic.
- **The graph is never mutated.** Each contract builds a `GraphView` with its
  excluded imports: kinds, suppressions, `ignore_imports` and baseline.
- **Output must be deterministic.** Sort before rendering. UI snapshots enforce
  this.
- **All log output goes to stderr**; reports go to stdout.

## Tests

- Unit tests live next to the code. The grimp algorithm tests are in
  `zenith_graph`.
- UI tests live in `tests/cases/<group>/<case>/`. Each case has a project, an
  `args` file (one invocation per line), an optional `fixture` naming a project
  under `tests/fixtures/`, and the recorded `output.snap`. The harness is
  `crates/zenith/tests/ui.rs`. It copies each case into a temp dir outside the
  repo, so the repo's own `.gitignore` cannot mask bugs.
- `tests/fixtures/acme` is a synthetic monorepo mirroring real-world patterns.
  Its `VIOLATIONS.md` documents every expected finding: keep it in sync with
  the fixture.
- When you change output, re-record the snapshots and **read the diff**. The
  snapshot is the assertion.
- Real-world runs: `just corpus <root> <config>`. Corpus configs live in the
  gitignored `corpus/` directory. **Never commit details of private
  codebases** (paths, package names) into the repo.

## Conventions

- Every file starts with a `//!` module doc. Public items and struct fields
  get `///` docs.
- Errors use `anyhow` in the binary and `String` or typed errors in libraries.
  Configuration errors name the contract they belong to.
- Use British spelling in prose (colour, normalise, behaviour), matching the
  user's other tools (`teamsearch`, `bracketlint`).
- Commits are conventional (`feat(contracts): …`, `fix(resolve): …`).
  Branches are `feds01/<topic>`.
