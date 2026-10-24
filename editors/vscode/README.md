# zenith for VS Code

[zenith](https://feds01.github.io/zenith/) checks which parts of a monorepo may
import which others, by the contracts in its `zenith.toml`. This extension
shows where the code breaks them, as you save, on the imports that do.

- Each import that breaks a contract is an error or a warning, as the
  contract's severity says, with the contract's guidance. The rest of an
  import chain is its related information.
- A mistake in `zenith.toml` is shown where it is, until it is fixed.
- A suppression that has no effect is faded.
- Each folder of a multi-root workspace with a `zenith.toml`, in it or above
  it, is checked. Folders under the same `zenith.toml`, such as the packages
  of a monorepo, share a server, which shows each finding once.

The extension runs `zenith lsp`, zenith's language server, which checks the
whole project whenever a file is saved: see
[editor integration](https://feds01.github.io/zenith/editors.html) in the
guide.

## The binary

The extension runs the first of these that it finds, and that has
`zenith lsp`, which a zenith that a project pins may predate:

1. the binary that the `zenith.path` setting names;
2. the project's own, if it has `@feds01/zenith` among its npm dependencies,
   so that it runs the version that the project pins;
3. the binary that comes with the extension, on macOS, Linux (ARM64 and
   x86-64) and Windows (x86-64);
4. `zenith` on the `PATH`.

Elsewhere, [install zenith](https://feds01.github.io/zenith/installation.html)
and put it on the `PATH`, or set `zenith.path`.

It runs only in a trusted workspace: the project's binary is the
workspace's own, and zenith runs tools such as `cargo` and `git` in it.

## Settings

| Setting               |                                                                                                                                                                                                                         |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `zenith.path`         | The path of the `zenith` binary to run, absolute or relative to the workspace folder.                                                                                                                                   |
| `zenith.args`         | More arguments for `zenith lsp`, such as `["--config", "tools/zenith.toml"]` for a configuration file that isn't in the folder or above it. A folder is also checked when they name its configuration file or its root. |
| `zenith.trace.server` | Log the messages between VS Code and the server: `off`, `messages` or `verbose`.                                                                                                                                        |

## Commands

- **zenith: Restart the server**, which finds the binary and the
  configuration anew.
- **zenith: Show the server's output**, its log, which says which binary it
  runs and why a check failed.
