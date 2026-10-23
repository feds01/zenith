# Editor integration

`zenith lsp` is a language server: an editor starts it, and it shows what
the contracts find as diagnostics on the code, for the people and the agents
that read them.

- When the editor starts it, and whenever the editor saves a file, it checks
  the whole project, as `zenith check` does, and publishes a diagnostic on
  each import that breaks a contract, a finding each as in the
  [compact](output.md#compact) format.
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

- Of an editor's workspace folders, only the first is checked.
- Each save is checked in turn, so saving several files in quick succession
  checks the project once for each.

## VS Code

zenith has no extension of its own yet. A generic client such as
[Generic LSP Client](https://marketplace.visualstudio.com/items?itemName=llllvvuu.llllvvuu-glspc)
starts it for one language, in `settings.json`:

```json
{
  "glspc.serverCommand": "zenith",
  "glspc.serverCommandArguments": ["lsp"],
  "glspc.languageId": "typescript"
}
```

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
