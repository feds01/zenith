# Editor integration

`zenith lsp` is a language server: an editor starts it, and it shows what
the contracts find as diagnostics on the code, for the people and the agents
that read them.

- When the editor starts it, and whenever the editor saves a file, it checks
  the whole project, as `zenith check` does, and publishes a diagnostic on
  each import that breaks a contract, a finding each as in the
  [compact](output.md#compact) format.
- It keeps the project's import graph from one check to the next. If the
  editor reports the changes to files, as VS Code does, a save reads and
  resolves again only the source files that changed since the last check:
  the one saved, and those that changed outside the editor, such as when a
  branch is checked out, once the editor reports them. Then it checks every
  contract again. A new or deleted file, or a change to a file that isn't a
  source file, such as `zenith.toml`, a manifest or a `tsconfig.json`, has
  the project read from scratch. So does every save in an editor that
  doesn't report changes, or when the project's root or `zenith.toml` is
  above the folder that the editor opened, as an editor reports only the
  changes in it.
- A diagnostic is an error or a warning as the contract's severity says, has
  the contract's id as its code, says how to fix the finding, as the
  compact format does, and has the contract's guidance after it. The
  rest of an import chain, or the other imports of a dependency that would
  break a cycle, are its related information.
- Stale baseline entries, and `ignore_imports` entries that match nothing,
  are diagnostics on the baseline and the configuration file. A finding
  that points at no file, such as a module that an exhaustive `layers`
  contract doesn't declare or a [limit](contracts/limits.md) that a module
  exceeds, is counted in a message that points at `zenith
  check`, shown when those findings change rather than on every save.
- A [suppression](suppressions.md) that has no effect is a hint on its
  comment, which editors fade.
- A file's diagnostics are cleared once it has none.
- A check that can't run keeps the diagnostics of the last one that did. A
  mistake in the configuration is an error where it is in `zenith.toml`,
  until it is fixed and saved; anything else that stops the check, even a
  crash, is shown as a message, and the server carries on.
- It checks what is saved, not what is being typed, and offers no
  completions or code actions.

It finds the project as `zenith check` would in the folder that the editor
opened, or in its working directory if the editor names no folder. It takes
the [options](cli.md) of every command, such as `--config` and `--no-cache`,
and logs to stderr, which editors keep in a log of the server.

Known limits:

- Of an editor's workspace folders, only the first is checked. The VS Code
  extension runs a server for each project that its folders are in.
- Each save is checked in turn, so saving several files in quick succession
  checks the project once for each.

## VS Code

Install the [Zenith Linter extension](https://marketplace.visualstudio.com/items?itemName=feds01.zenith)
from the Marketplace, or from [Open VSX](https://open-vsx.org/extension/feds01/zenith)
in editors that use it, such as Cursor and VSCodium. It checks each folder of
the workspace that has a `zenith.toml`, in it or in a folder above it, and
starts or stops checking a folder as it gains or loses one. It runs a server
for each project: folders under the same `zenith.toml`, such as the packages
of a monorepo, share one, which shows each finding once.

It runs the first of these binaries that it finds:

1. the one that the `zenith.path` setting names, absolute or relative to the
   folder;
2. the project's own, if it has `@feds01/zenith` among its
   [npm](installation.md#npm) dependencies, so that the editor runs the
   version that the project pins, unless it is 0.5.0 or older, which have no
   `zenith lsp`;
3. the one that comes with the extension, for macOS, Linux (ARM64 and
   x86-64) and Windows (x86-64);
4. `zenith` on the `PATH`.

It runs only in a trusted workspace, as the project's binary is the
workspace's own, and zenith runs tools such as `cargo` and `git` in it.

| Setting | |
|---|---|
| `zenith.path` | The path of the binary to run. |
| `zenith.args` | More arguments for `zenith lsp`, such as `["--config", "tools/zenith.toml"]`. A folder is also checked when they name its configuration file or its root. |
| `zenith.trace.server` | Logs the messages between VS Code and the server: `off`, `messages` or `verbose`. |

**zenith: Restart the server** finds the binary and the configuration anew,
and **zenith: Show the server's output** shows its log, which says which
binary it runs.

## Neovim

With Neovim 0.11 or later, in `init.lua`:

```lua
vim.lsp.config("zenith", {
  cmd = { "zenith", "lsp" },
  filetypes = { "javascript", "javascriptreact", "typescript", "typescriptreact",
                "go", "python", "rust", "terraform", "hcl" },
  root_markers = { "zenith.toml" },
})
vim.lsp.enable("zenith")
```

## Helix

In `languages.toml`, add the server, then list it among the servers of each
language, after those that the language already uses:

```toml
[language-server.zenith]
command = "zenith"
args = ["lsp"]

[[language]]
name = "typescript"
roots = ["zenith.toml"]
language-servers = ["typescript-language-server", "zenith"]
```
