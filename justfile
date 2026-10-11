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

# Run the CLI from source in the current directory, e.g. `just run version`
[no-cd]
run *args:
    cargo run -q -p zenith -- "$@"

# Run all tests, extra args go to `cargo test`, e.g. `just test -p zenith`
test *args:
    cargo test "$@"

# Regenerate zenith.schema.json from the config types
schema:
    cargo build -q -p zenith
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

# Run the tests, clippy and the formatting check (CI also runs `cargo deny`)
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

    fail() {
        echo "error: $1" >&2
        exit 1
    }

    # Releases are cut from an up-to-date, clean main.
    branch="$(git rev-parse --abbrev-ref HEAD)"
    if [ "$branch" != "main" ]; then
        fail "releases are cut from main (on '$branch')"
    fi
    if [ -n "$(git status --porcelain)" ]; then
        fail "the working tree is not clean"
    fi
    git fetch origin main --tags --quiet
    if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
        fail "main is not in sync with origin/main"
    fi

    # The version comes from the binary crate, and must not be tagged yet.
    version="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/zenith/Cargo.toml | head -1)"
    tag="v$version"
    if git rev-parse --quiet --verify "refs/tags/$tag" >/dev/null; then
        fail "tag $tag already exists, bump the version first"
    fi

    read -r -p "Release $tag? [y/N] " answer
    case "$answer" in
        y | Y) ;;
        *)
            echo "aborted"
            exit 1
            ;;
    esac

    git tag -a "$tag" -m "Release $tag"
    git push origin "$tag"
