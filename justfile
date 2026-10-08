# Zenith development commands. Run `just` to list them.

set positional-arguments

# List available recipes
default:
    @just --list --unsorted

# Install the pinned toolchain and the pre-commit hooks
setup:
    rustup toolchain install
    @command -v pre-commit >/dev/null || { echo "pre-commit not found, install it with 'brew install pre-commit' or 'pipx install pre-commit'"; exit 1; }
    pre-commit install

# Install the `zenith` binary from this checkout into ~/.cargo/bin
install:
    cargo install --locked --path crates/zenith

# Remove the installed `zenith` binary
uninstall:
    cargo uninstall zenith

# Build the workspace
build:
    cargo build --workspace

# Build an optimised `zenith` binary into target/release
build-release:
    cargo build --release -p zenith

# Run the CLI from source in the current directory, e.g. `just run check`
[no-cd]
run *args:
    cargo run -q -p zenith -- "$@"

# Run all tests, extra args go to `cargo test`, e.g. `just test -p zenith_graph`
test *args:
    cargo test --workspace "$@"

# Run only the UI snapshot tests in tests/cases, optionally filtered by name
test-ui *filter:
    cargo test -p zenith --test ui -- "$@"

# Re-generate the `.stdout`/`.stderr` snapshots in tests/cases from current output
update-snapshots *filter:
    REGENERATE_OUTPUT=1 cargo test -p zenith --test ui -- "$@"

# Run a release build of `zenith check` against a local checkout, e.g. `just corpus ~/code/acme corpus/acme.toml`
corpus path config *args:
    cargo build --release -p zenith
    ./target/release/zenith check --config "{{config}}" --show-timings "${@:3}"

# Regenerate zenith.schema.json from the config types
schema:
    cargo run -q -p zenith -- schema > zenith.schema.json

# Run clippy, denying warnings
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check formatting without changing files
fmt-check:
    cargo fmt --all -- --check

# Run clippy and the formatting check
lint: clippy fmt-check

# Run everything CI runs: tests, clippy and the formatting check
ci: test lint

# Build and open the API docs for the workspace crates
docs:
    cargo doc --workspace --no-deps --open

# Remove build artifacts
clean:
    cargo clean

# Tag and push a release from main, using the version in crates/zenith/Cargo.toml
release:
    #!/usr/bin/env bash
    set -euo pipefail
    branch="$(git rev-parse --abbrev-ref HEAD)"
    if [ "$branch" != "main" ]; then echo "error: releases are cut from main (on '$branch')"; exit 1; fi
    if [ -n "$(git status --porcelain)" ]; then echo "error: the working tree is not clean"; exit 1; fi
    git fetch origin main --quiet
    if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then echo "error: main is not in sync with origin/main"; exit 1; fi
    version="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/zenith/Cargo.toml | head -1)"
    if git rev-parse "v$version" >/dev/null 2>&1; then echo "error: tag v$version already exists, bump the version first"; exit 1; fi
    read -r -p "Release v$version? [y/N] " answer
    if [ "$answer" != "y" ]; then echo "aborted"; exit 1; fi
    git tag -a "v$version" -m "Release v$version"
    git push origin "v$version"
