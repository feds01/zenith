<p align="center"><img src="docs/logo/zenith-icon.svg" width="96" alt=""></p>

# Zenith

> A language-agnostic architecture linter for monorepos.

Zenith checks which parts of a codebase may import which others.

You declare *contracts*, such as layers, forbidden dependencies or public entry points, in
`zenith.toml`, and `zenith check` fails when the code breaks one, pointing at
the import, or the chain of imports, that does.

If you have used [import-linter](https://github.com/seddonym/import-linter) in
Python, the contracts will feel familiar: Zenith brings the same ideas to
monorepos, whatever their languages.

## Installation

See [INSTALL.md](INSTALL.md).

## Documentation

The [user guide](https://feds01.github.io/zenith/) covers the contracts, the
configuration and the command line.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT
