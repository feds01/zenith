# Command line

```text
zenith check [<path>…] [--contract <id>]… [--format pretty|compact|json|github]
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
| `check` | Checks the contracts, or only those of `--contract`. With paths, it only reports what involves the files under them. |
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
| `--tracked` | Only scan the files that git tracks, leaving out untracked ones. Use it in pre-commit hooks, where untracked files aren't part of the commit. Outside a git repository, it is an error. |
| `--no-cache` | Read and parse every file, without reading or writing the [cache](configuration.md#cache). |
| `--color auto\|always\|never` | Whether to colour the output. By default, only when it goes to a terminal and `NO_COLOR` isn't set. |
| `-v`, `-vv` | Show more of what zenith is doing, on stderr. |

Arguments may also come from a file, with `@path`. Selectors are matched by
zenith itself, so quote them, as in `zenith graph "web/src/*"`.

## Checking some files

`zenith check <path>…` still checks the whole project, but only reports what
involves the files under the paths, and only fails if that is broken. That
suits an agent or an editor checking the files it has just changed:

```sh
zenith check --format compact src/web/orders.ts
```

What involves a file:

- a violation's chain or import, if any import on it is from or into the file;
- an `acyclic` cycle, if one of the imports it suggests removing is;
- an undeclared module of an exhaustive `layers` contract, if the file is in it;
- a stale baseline entry, an unused suppression, an unresolved import or a
  file that can't be read, of the file.

Unmatched `ignore_imports` entries are about the configuration, not a file,
so they are always reported. A path is relative to the working directory and
must be inside the root, but needn't exist, so a deleted file's importers
still count.

A cycle is reported on the imports zenith suggests removing, so one that a
change closes can be left out if those imports are elsewhere. A full
`zenith check`, as in CI, still finds it.

## Exit codes

| Code | |
|---|---|
| 0 | Every contract of `error` severity is kept. |
| 1 | A contract of `error` severity is broken, or has stale baseline entries that are errors. |
| 2 | The configuration or the command line is invalid, or something went wrong. |

`zenith resolve` exits with 1 when the specifier doesn't resolve.
