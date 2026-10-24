# Installation

zenith is a single binary, for macOS, Linux and Windows. Every
[GitHub release](https://github.com/feds01/zenith/releases) has prebuilt
binaries and installers, and is published to npm, to a Homebrew tap, and
as a VS Code extension.

## npm

To add zenith to a project's dev dependencies, and run it with `npx`:

```sh
npm install --save-dev @feds01/zenith
npx zenith check
```

Install it with `--global` to have `zenith` everywhere.

The binary for each platform is in a package of its own, such as
`@feds01/zenith-darwin-arm64` or `@feds01/zenith-linux-x64`, which
`@feds01/zenith` lists as optional dependencies, so the package manager
installs only the one for your platform. There is no install script, and
nothing is downloaded when zenith first runs:

- bun installs it without an entry in `trustedDependencies`;
- the lockfile's integrity hashes cover the binary;
- it installs offline from the package manager's cache.

There are packages for macOS (Apple silicon and Intel), Linux (ARM64 and
x86-64, static binaries that run with glibc or musl) and Windows (x86-64).
Leaving optional dependencies out, e.g. with `npm install --omit=optional`,
leaves out the binary too, and `zenith` then says which package to add. On
other platforms, build zenith [from source](#from-source) and set
`ZENITH_BINARY` to the path of the binary.

## Homebrew

```sh
brew install feds01/tap/zenith
```

Use the full name: `brew install zenith` installs a different tool of the same
name, from Homebrew's core formulae.

## Shell installer

On macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/feds01/zenith/releases/latest/download/zenith-installer.sh | sh
```

## PowerShell installer

On Windows:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/feds01/zenith/releases/latest/download/zenith-installer.ps1 | iex"
```

## Prebuilt binaries

Download the archive for your platform from the
[latest release](https://github.com/feds01/zenith/releases/latest), unpack it,
and put `zenith` on your `PATH`.

## From source

With a Rust toolchain:

```sh
cargo install --locked --git https://github.com/feds01/zenith zenith
```

## VS Code

The [VS Code extension](editors.md#vs-code)
comes with zenith, and runs a project's own zenith when it has one, from npm.
It shows where the code breaks its contracts, as you save.

## Check the installation

```sh
zenith version
```
