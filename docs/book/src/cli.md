# Command line

```text
zenith check [--contract <id>]… [--format pretty|compact|json|github]
             [--no-baseline] [--show-unresolved] [--show-timings]
zenith baseline [--contract <id>]… [--prune]
zenith graph [<selector>] [--depth <n>] [--format tree|dot|json] [--exclude-kind <kind>]…
zenith why <from-selector> <to-selector> [--exclude-kind <kind>]… [--limit <n>]
zenith resolve <file> <specifier>
zenith init [--force]
zenith version
```

| Command | |
|---|---|
| `check` | Checks the contracts, or only those of `--contract`. |
| `baseline` | Records the current violations in the [baseline](baseline.md). |
| `graph` | Shows the module tree under a selector, and the dependencies between each node's children, as a tree, Graphviz or JSON. |
| `why` | Shows the shortest import chains from one selector to another. |
| `resolve` | Shows what a specifier resolves to from a file, and why. |
| `init` | Writes a starter `zenith.toml` for the workspace packages it finds. |

Every command takes these options:

| Option | |
|---|---|
| `--config <path>` | The configuration file, instead of the nearest `zenith.toml`. |
| `--root <dir>` | The project root, instead of the configuration file's directory. |
| `--tracked` | Only scan the files that git tracks, leaving out untracked ones. Use it in pre-commit hooks, where untracked files aren't part of the commit. |
| `--color auto\|always\|never` | Whether to colour the output. By default, only when it goes to a terminal and `NO_COLOR` isn't set. |
| `-v`, `-vv` | Show more of what zenith is doing, on stderr. |

Arguments may also come from a file, with `@path`. Selectors are matched by
zenith itself, so quote them, as in `zenith graph "web/src/*"`.

## Exit codes

| Code | |
|---|---|
| 0 | Every contract of `error` severity is kept. |
| 1 | A contract of `error` severity is broken, or has stale baseline entries that are errors. |
| 2 | The configuration or the command line is invalid, or something went wrong. |

`zenith resolve` exits with 1 when the specifier doesn't resolve.
