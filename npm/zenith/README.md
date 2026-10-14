# zenith

An architecture linter for TypeScript and JavaScript codebases, built for
monorepos. Zenith enforces *contracts* about which parts of a codebase may
import which other parts, declared in `zenith.toml`, and `zenith check` fails
when the code breaks them.

```sh
npm install --save-dev @feds01/zenith
npx zenith check
```

See the [user guide](https://feds01.github.io/zenith/) for the contracts, the
configuration and the command line.

## How it is installed

Zenith is a native binary. This package has none itself, nor any install
script: the binary for each platform is in a package of its own, which this
one lists as an optional dependency, and the package manager installs only
the one for your platform:

| Package | Platform |
|---|---|
| `@feds01/zenith-darwin-arm64` | macOS, Apple silicon |
| `@feds01/zenith-darwin-x64` | macOS, Intel |
| `@feds01/zenith-linux-arm64` | Linux, ARM64 (static, any libc) |
| `@feds01/zenith-linux-x64` | Linux, x86-64 (static, any libc) |
| `@feds01/zenith-win32-x64` | Windows, x86-64 |

So it installs offline from a package manager's cache, the lockfile's
integrity hashes cover the binary, and bun needs no `trustedDependencies`
entry for it. Don't leave optional dependencies out of the install, e.g. with
`npm install --omit=optional`, or the `zenith` command has no binary to run.

On another platform, build zenith from source with
`cargo install --locked --git https://github.com/feds01/zenith zenith`, and
set `ZENITH_BINARY` to the path of the binary.

## Licence

MIT
