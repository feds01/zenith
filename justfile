# Zenith development commands. Run `just` to list them.

set positional-arguments

# List available recipes
default:
    @just --list --unsorted

# Install the pinned toolchain and the pre-commit hooks
setup:
    rustup toolchain install
    @command -v sccache >/dev/null || echo "sccache not found: with it, worktrees share their compiled dependencies. Install it with 'brew install sccache' or 'cargo install sccache --locked', and set 'build.rustc-wrapper = \"sccache\"' in ~/.cargo/config.toml"
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

# Measure the current branch with CodSpeed in CI, once it is pushed, where the counts compare with main's
bench-ci:
    #!/usr/bin/env sh
    set -eu
    branch="$(git branch --show-current)"
    git fetch -q origin "$branch"
    if [ "$(git rev-parse HEAD)" != "$(git rev-parse "origin/$branch")" ]; then
        echo "push $branch first: CI measures what is on GitHub" >&2
        exit 1
    fi
    gh workflow run codspeed.yml --ref "$branch"
    echo "Started the CodSpeed workflow on $branch; follow it with \`gh run watch\`."

# Measure a benchmark with CodSpeed's simulation in a Linux container, e.g. `just bench-instructions contracts layers`; set CODSPEED_BASE to a run's id to compare with it
bench-instructions bench *filter:
    #!/usr/bin/env sh
    set -eu
    shift
    docker build -q -t zenith-codspeed crates/zenith_benchmark/codspeed >/dev/null
    # CodSpeed reads the repository to tell which commit it measured. A
    # worktree's .git points into the main checkout, so both are mounted where
    # they are here.
    root="$(git rev-parse --show-toplevel)"
    git_dir="$(git rev-parse --path-format=absolute --git-common-dir)"
    # Fat LTO needs more memory than Docker usually has, so it is left out: the
    # counts compare with each other, but not with CI's, which also runs on
    # x86_64. Use `just bench-ci` for those.
    docker run --rm -v "$root":"$root" -v "$git_dir":"$git_dir":ro -w "$root" \
        -v zenith-codspeed-config:/root/.config/codspeed \
        -v zenith-codspeed-cargo:/usr/local/cargo/registry \
        -v zenith-codspeed-rustup:/usr/local/rustup \
        -v zenith-codspeed-target:/target -e CARGO_TARGET_DIR=/target \
        -e CARGO_PROFILE_RELEASE_LTO=false -e CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
        -e RAYON_NUM_THREADS=1 -e MIMALLOC_PURGE_DELAY=-1 -e CODSPEED_TOKEN -e CODSPEED_BASE \
        zenith-codspeed sh -c '
            set -eu
            git config --global --add safe.directory "*"
            cargo codspeed build -m simulation -p zenith_benchmark --bench "$0" >/dev/null
            codspeed run -m simulation ${CODSPEED_BASE:+--base "$CODSPEED_BASE"} -- \
                cargo codspeed run -p zenith_benchmark --bench "$0" "$@" || {
                echo "If CodSpeed asks to log in, run \`just codspeed-login\` or set CODSPEED_TOKEN." >&2
                exit 1
            }' "{{bench}}" "$@"

# Log the container of `just bench-instructions` in to CodSpeed, once
codspeed-login:
    docker build -q -t zenith-codspeed crates/zenith_benchmark/codspeed >/dev/null
    docker run --rm -it -v zenith-codspeed-config:/root/.config/codspeed zenith-codspeed codspeed auth login

# Run clippy, denying warnings
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check formatting without changing files
fmt-check:
    cargo fmt --all -- --check

# Check zenith's own contracts, in zenith.toml
architecture:
    cargo run -q -p zenith -- check

# Run clippy, the formatting check and zenith's own contracts
lint: clippy fmt-check architecture

# Run the tests, clippy and the formatting check (CI also runs `cargo deny`)
ci: test lint

# Build and open the API docs for the workspace crates
docs:
    cargo doc --workspace --no-deps --open

# Serve the user guide in docs/book, rebuilt on every change; needs mdBook 0.4.40
book:
    mdbook serve docs/book --open

# Remove build artifacts
clean:
    cargo clean

# Tag and push a release from main, using the workspace version in Cargo.toml
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

    # The version comes from the workspace, and must not be tagged yet.
    version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
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
    echo "Once the release workflow has run, approve the npm package on npmjs.com, under Staged Packages."

# Commit the Homebrew formula of each release in $PLAN, dist's plan, to the tap checked out at `tap`, and push it (run by the release workflow)
[no-cd]
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

# Assemble and pack the npm packages into `out`, from the release archives in `archives`, at the workspace version in Cargo.toml unless given another, and only for `targets` if given any
[no-cd]
npm-packages archives out version="" *targets:
    #!/usr/bin/env bash
    set -euo pipefail
    shift 3
    version="{{version}}"
    if [ -z "$version" ]; then
        version="$(sed -n 's/^version = "\(.*\)"/\1/p' "{{justfile_directory()}}/Cargo.toml" | head -1)"
    fi
    # An absolute path: npm reads a relative one, like `out/zenith`, as a
    # GitHub repository.
    out="$(mkdir -p "{{out}}" && cd "{{out}}" && pwd)"
    node "{{justfile_directory()}}/npm/assemble.mjs" "$version" "{{archives}}" "$out" "$@"
    for package in "$out"/zenith*/; do
        npm pack --json --pack-destination "$out" "$package" | jq --raw-output '.[].filename'
    done

# Stage the npm packages of the release in $PLAN, dist's plan, from the archives in `dir`, for a maintainer to approve, skipping versions already on npm (run by the release workflow)
[no-cd]
publish-npm dir:
    #!/usr/bin/env bash
    set -euo pipefail
    # The version that the plan releases zenith at, if it releases it.
    version="$(jq --raw-output '.releases[] | select(.app_name == "zenith") | .app_version' <<<"$PLAN")"
    if [ -z "$version" ]; then
        echo "The release has no zenith to stage"
        exit 0
    fi

    # 1. Assemble the packages: one per platform, and @feds01/zenith.
    out="$(mktemp -d)"
    node "{{justfile_directory()}}/npm/assemble.mjs" "$version" "{{dir}}" "$out"

    # 2. Stage the platform packages first, then @feds01/zenith, which depends
    #    on them.
    for package in "$out"/zenith-*/ "$out/zenith/"; do
        # What it stages, e.g. `@feds01/zenith@0.1.0`.
        spec="$(jq --raw-output '.name + "@" + .version' "$package/package.json")"

        # A version already on npm, published by hand or approved after an
        # earlier run, is skipped, so that the job can be re-run. One that is
        # staged but not yet approved can't be staged again: approve or reject
        # it first.
        if [ -n "$(npm view "$spec" version 2>/dev/null)" ]; then
            echo "$spec is already on npm"
            continue
        fi

        # Staged, not published: it goes live once a maintainer approves it,
        # with 2FA. npm prints the id to approve it by, as
        # `+ @feds01/zenith@0.1.0 (staged with id <id>)`.
        tarball="$out/$(npm pack --json --pack-destination "$out" "$package" | jq --raw-output '.[].filename')"
        staged="$(npm stage publish --access public "$tarball")"
        echo "$staged"
        id="$(sed -n 's/.*(staged with id \(.*\))$/\1/p' <<<"$staged")"
        echo "::notice title=npm::Staged $spec. Approve it on npmjs.com, under Staged Packages, or with npm stage approve ${id:-<id>}."
    done
