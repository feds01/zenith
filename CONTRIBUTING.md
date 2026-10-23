# Contributing to Zenith

## Development

Zenith builds with a pinned nightly toolchain (see `rust-toolchain.toml`) and
uses [just](https://github.com/casey/just):

```sh
just setup     # install the toolchain and the pre-commit hooks
just test      # all tests
just lint      # clippy (deny warnings), the formatting check and zenith's own contracts
just run       # run the CLI from source, e.g. `just run version`
```

With [sccache](https://github.com/mozilla/sccache) as cargo's
`build.rustc-wrapper` in `~/.cargo/config.toml`, every checkout and worktree
shares its compiled dependencies; `just setup` says how to set it up.

Run `just lint` before committing; the pre-commit hooks run the same checks on
the files you commit. CI runs the tests on Linux, macOS and Windows, and the
formatting check, clippy, zenith's own contracts and `cargo deny` on Linux.

See [CLAUDE.md](CLAUDE.md) for the architecture and conventions.

## The user guide

The user guide is an [mdBook](https://rust-lang.github.io/mdBook/) in
[docs/book](docs/book/src/introduction.md). `just book` serves it, rebuilt on
every change; it needs mdBook 0.4.40, the version that CI builds it with.
Every change to it on `main` is published to
[GitHub Pages](https://feds01.github.io/zenith/).

## Releasing

Bump `version` in the root `Cargo.toml`, and with it `Cargo.lock`, and run
`just changelog <version>`, which writes [CHANGELOG.md](CHANGELOG.md) with
[git-cliff](https://git-cliff.org) from the titles of the pull requests
merged since the last release (see [cliff.toml](cliff.toml)). Merge that to
`main`, then run `just release` to tag it.
[cargo-dist](https://opensource.axo.dev/cargo-dist/) builds the binaries and
installers, and publishes the GitHub release, with the release's section of
the changelog as its notes, and the Homebrew formula.

A pull request's title is its entry in the changelog, so write it as a
conventional commit, as in `feat(graph): …`. Features, fixes, performance and
documentation are listed; other types are not.

The npm packages are built from the release's archives by `just publish-npm`,
from the templates in [npm/](npm): a package per platform with just its
binary, such as `@feds01/zenith-darwin-arm64`, and `@feds01/zenith`, which
depends on them all, optionally, and runs the right one. `just npm-packages`
builds them locally. They are staged rather than published: approve them on
npmjs.com, under Staged Packages, or with `npm stage approve <id>` (npm 11.15
or later), the platform packages before `@feds01/zenith`. The release run
shows the ids, and so does `npm stage list <package>`.
