# Installation

zenith is a single binary, for macOS, Linux and Windows. Every
[GitHub release](https://github.com/feds01/zenith/releases) has prebuilt
binaries and installers, and is published to npm and to a Homebrew tap.

## npm

To add zenith to a project's dev dependencies, and run it with `npx`:

```sh
npm install --save-dev @feds01/zenith
npx zenith check
```

The package installs the binary for your platform. Install it with `--global`
to have `zenith` everywhere.

## Homebrew

```sh
brew install feds01/tap/zenith
```

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
cargo install --git https://github.com/feds01/zenith zenith
```

## Check the installation

```sh
zenith version
```
