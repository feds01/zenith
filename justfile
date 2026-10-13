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

# Extra args replace the default command, `check --show-timings`.
# Run a release build on a local checkout with a config from corpus/, e.g. `just corpus ~/code/app corpus/app.toml`
corpus path config *args:
    #!/usr/bin/env sh
    set -eu
    shift 2
    [ $# -gt 0 ] || set -- check --show-timings
    cargo run -q --release --locked -p zenith -- --root "{{path}}" --config "{{config}}" "$@"

# Run all tests, extra args go to `cargo test`, e.g. `just test -p zenith`
test *args:
    cargo test "$@"

# Run only the UI snapshot tests in tests/cases, optionally filtered by name
test-ui *filter:
    cargo test -p zenith --test ui -- "$@"

# Re-record the `output.snap` snapshots in tests/cases from the current output
update-snapshots *filter:
    REGENERATE_OUTPUT=1 cargo test -p zenith --test ui -- "$@"

# Regenerate zenith.schema.json from the config types
schema:
    cargo build -q -p zenith
    cargo run -q -p zenith -- schema > zenith.schema.json

# Run the benchmarks, extra args go to divan, e.g. `just bench walk`
bench *args:
    cargo bench -p zenith_benchmark -- "$@"

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

# Commit the Homebrew formula of each release in $PLAN, dist's plan, to the tap checked out at `tap`, and push it (run by the release workflow)
publish-formula tap:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{tap}}"
    git config user.name "github-actions[bot]"
    git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
    export PATH="/home/linuxbrew/.linuxbrew/bin:$PATH"
    brew update

    # Each release with a formula, as a line of JSON. Its formula may be named
    # other than the app.
    releases="$(jq --compact-output '.releases[] | select(any(.artifacts[]; endswith(".rb")))' <<<"$PLAN")"
    while IFS= read -r release; do
        [ -n "$release" ] || continue
        formula="$(jq --raw-output '.artifacts[] | select(endswith(".rb"))' <<<"$release")"
        version="$(jq --raw-output '.app_version' <<<"$release")"

        # Fix the formula's style, without reformatting its description or homepage.
        brew style --except-cops FormulaAudit/Homepage,FormulaAudit/Desc,FormulaAuditStrict \
            --fix "Formula/$formula" || true

        git add "Formula/$formula"
        git commit --message "${formula%.rb} $version"
    done <<<"$releases"

    git push
