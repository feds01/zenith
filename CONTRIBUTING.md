# Contributing to Zenith

## Development

Zenith builds with a pinned nightly toolchain (see `rust-toolchain.toml`) and
uses [just](https://github.com/casey/just):

```sh
just setup     # install the toolchain and the pre-commit hooks
just test      # all tests
just lint      # clippy (deny warnings) and the formatting check
just run       # run the CLI from source, e.g. `just run version`
```

Run `just lint` before committing. CI runs the tests on Linux, macOS and
Windows, and the formatting check, clippy and `cargo deny` on Linux.

See [CLAUDE.md](CLAUDE.md) for the architecture and conventions.

## The user guide

The user guide is an [mdBook](https://rust-lang.github.io/mdBook/) in
[docs/book](docs/book/src/introduction.md). `just book` serves it, rebuilt on
every change; it needs mdBook 0.4.40, the version that CI builds it with.
Every change to it on `main` is published to
[GitHub Pages](https://feds01.github.io/zenith/).

## Releasing

Bump `version` in `crates/zenith/Cargo.toml`, and with it `Cargo.lock`, merge
to `main`, then run `just release` to tag it.
[cargo-dist](https://opensource.axo.dev/cargo-dist/) builds the binaries and
installers, and publishes the GitHub release and the Homebrew formula. The npm
package is staged rather than published: approve it on npmjs.com, under
Staged Packages, or with `npm stage approve <id>` (npm 11.15 or later). The
release run shows the id, and so does `npm stage list @feds01/zenith`.
