<p align="center"><img src="docs/logo/zenith-icon.svg" width="96" alt=""></p>

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
workspace packages, `tsconfig` path aliases, `exports` maps, type-only imports,
and `"use client"` and `"use server"` boundaries. Zenith is a single Rust
binary built on [oxc](https://oxc.rs).

## Installation

See [INSTALL.md](INSTALL.md).

## Documentation

The [user guide](https://feds01.github.io/zenith/) covers the contracts, the
configuration and the command line.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT
