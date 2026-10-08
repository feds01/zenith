# Zenith

> An architecture linter for TypeScript and JavaScript codebases, built for monorepos.

Zenith enforces *contracts* about which parts of a codebase may import which
other parts, such as layers, forbidden dependencies, independent modules,
protected modules, acyclic packages and public entry points. You declare them
in `zenith.toml`, and `zenith check` fails when the code breaks them, pointing
at the exact import, or the full import chain when the dependency is indirect.

The contract model comes from Python's
[import-linter](https://github.com/seddonym/import-linter), and the graph
algorithms are ports of its engine [grimp](https://github.com/python-grimp/grimp).
Both are adapted to how JS/TS projects are organised: files and directories,
workspace packages, `tsconfig` path aliases, `exports` maps, type-only imports
and `"use client"` boundaries. Zenith is a single Rust binary built on
[oxc](https://oxc.rs).

## Status

Zenith is under construction. Its build spec and its implementation land on
`main` one reviewed pull request at a time.

## Installation

The repository is private for now, so these need access to it.

From source:

```sh
cargo install --git https://github.com/feds01/zenith zenith
```

Prebuilt binaries and installers will be attached to
[GitHub releases](https://github.com/feds01/zenith/releases) once the first
version is tagged.

## Development

Zenith builds with a pinned nightly toolchain (see `rust-toolchain.toml`) and
uses [just](https://github.com/casey/just):

```sh
just setup     # install the toolchain and the pre-commit hooks
just test      # all tests
just lint      # clippy (deny warnings) and the formatting check
just run       # run the CLI from source, e.g. `just run version`
```

See [CLAUDE.md](CLAUDE.md) for the architecture and conventions.

## Releasing

Bump `version` in `crates/zenith/Cargo.toml`, merge to `main`, then run
`just release` to tag it. [cargo-dist](https://opensource.axo.dev/cargo-dist/)
builds the binaries and installers and publishes the GitHub release.

## License

MIT
