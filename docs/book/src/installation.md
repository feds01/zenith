# Installation

Prebuilt binaries are attached to every
[GitHub release](https://github.com/feds01/zenith/releases), for macOS, Linux
and Windows. The same release is published to npm and to a Homebrew tap:

```sh
# npm, for a project's dev dependencies or globally
npm install --save-dev @feds01/zenith
npx zenith check

# Homebrew
brew install feds01/tap/zenith

# The shell installer, for macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/feds01/zenith/releases/latest/download/zenith-installer.sh | sh
```

From source, with a Rust toolchain:

```sh
cargo install --git https://github.com/feds01/zenith zenith
```

Check that it works with `zenith version`.
