# Command line

```text
zenith check [<path>…] [--since <ref>] [--contract <id>]…
             [--format pretty|compact|json|github|sarif]
             [--no-baseline] [--show-unresolved] [--show-timings]
zenith baseline [--contract <id>]… [--prune]
zenith graph [<selector>] [--depth <n>] [--format tree|dot|mermaid|json] [--exclude-kind <kind>]…
zenith graph diff <base> [<head>] [--depth <n>] [--format tree|json] [--exclude-kind <kind>]…
             [-- <selector>]
zenith why <from-selector> <to-selector> [--exclude-kind <kind>]… [--limit <n>]
zenith resolve <file> <specifier>
zenith init [--force]
zenith suggest
zenith version
```

| Command | |
|---|---|
| `check` | Checks the contracts, or only those of `--contract`. With paths, or [`--since`](#checking-what-a-branch-changed), it only reports what involves the files under them. `--format` picks one of the [output formats](output.md), such as `sarif` for code scanning. |
| `baseline` | Records the current violations in the [baseline](baseline.md). |
| `graph` | Shows the module tree under a selector, and the dependencies between each node's children, as a tree, Graphviz, Mermaid or JSON. |
| `graph diff` | Shows the dependencies between a selector's children that were added and removed from one git ref to another. See [Comparing refs](#comparing-refs). |
| `why` | Shows the shortest import chains from one selector to another. |
| `resolve` | Shows what a specifier resolves to from a file, and why. |
| `init` | Writes a starter `zenith.toml` for the workspace packages it finds. |
| `suggest` | Prints contracts that the code already keeps, as TOML: see [suggesting contracts](#suggesting-contracts). |

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

When a `graph` selector matches several modules, each is shown on its own,
except in Mermaid, where each is a subgraph of one flowchart, so that the
output renders from a single code block.

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

To report on the files that a branch changed, without listing them, use
[`--since`](#checking-what-a-branch-changed).

## Checking what a branch changed

`zenith check --since <ref>` reports as if the paths were every file that the
branch changed since it left `<ref>`: every file that differs between the
merge base of `<ref>` and `HEAD` and the working tree, as in
`git diff <ref>...` and a pull request's diff. So what has landed on `<ref>`
since the branch left it is not reported. The files are those added, copied,
modified, renamed or deleted, a renamed file under both its names, and the
untracked files that git doesn't ignore, unless `--tracked` is given. `<ref>`
is anything git names a commit by, such as a branch, a tag, `HEAD~1` or a
hash:

```sh
zenith check --since main --format github
```

With paths too, it reports on the files under either. A ref that names no
commit, as in a shallow clone that hasn't fetched it, and a project outside a
git repository, are errors. Where git finds no merge base, as in a shallow
clone that hasn't fetched enough history to reach the merge base, the working
tree is compared with `<ref>` itself, with a warning.

## Code scanning

`zenith check --format sarif > zenith.sarif` writes a SARIF log, for GitHub
code scanning and other dashboards, which is then uploaded, as in
[the workflow of the `sarif` format](output.md#sarif). Its paths are relative
to the root of the git repository, so a project in a subdirectory of one works
too. The exit code is the same as with any other format.

## Suggesting contracts

`zenith suggest` reads the import graph and prints, as TOML, contracts that
the code keeps as it is, each with a comment that says why. They are about
the workspace packages, or the top-level directories when there are fewer
than two packages:

- `acyclic`, when they depend on each other in no cycle. The packages are
  written as `pkg:*`, so it covers the packages added later too, unless a
  package is inside another;
- `layers`, when those that depend on each other stack in one strict order,
  each depending on the next;
- `independence`, for the packages in one directory, such as `apps`, when none
  of them depends on another.

```toml
# The workspace packages in `apps` do not depend on each other.
[contracts.apps-independent]
type = "independence"
modules = ["pkg:@acme/admin", "pkg:@acme/web"]
```

Each suggestion is checked against the graph before it is printed, as a
contract of the configuration, with its `[analysis]` defaults, so it passes
`zenith check` from the start.

It never writes a file. Write the suggestions to one to review them, as in
`zenith suggest > suggested.toml`, and copy the ones you want into
`zenith.toml`; or let an agent read them. A contract whose id the
configuration already has is not suggested, so once you have added them, a
second run only prints what is new.

## Comparing refs

`zenith graph diff <base> [<head>]` answers "how did the architecture change
in this branch?". It builds the graph at the merge base of the refs `<base>`
and `<head>`, or `HEAD` without one, and at `<head>`, or of the working tree
without one, and shows the dependencies between the children of the selector
that were added (`+`) and removed (`-`). A ref is anything git names a commit
by, such as `main` or `HEAD~3`, as for `--since`:

```console
$ zenith graph diff main -- web/src
dependencies between the children of `web/src` that changed:
  + web/src/app -> web/src/server (2 imports)
  - web/src/lib -> web/src/types.ts (1 import)
```

As in `git diff <base>...` and a pull request's diff, the base is where the
branch left `<base>`, so what has landed on `<base>` since is not shown as the
branch's change. Where git finds no merge base, as in a shallow clone, the
tip of `<base>` is compared instead, with a warning.

As in `git diff`, the selector comes after `--`, so that it can't be taken for
a ref, and the options go before it. It defaults to the whole project, and
need only match at one of the refs, so a package that the branch adds shows
every dependency between its children as added.

`--depth` means something else here than in `graph`, where it is how deep
the tree is drawn: it is how far below the selector to compare; 1 compares
its children. `--depth 2` compares the children of the children instead, and
so on. `--exclude-kind` leaves imports out as in `graph`. `--format json`
gives each node of the selector as `{"module", "added", "removed"}`, and each
dependency as `{"from", "to", "imports"}`.

A file that git sees renamed keeps its dependencies under its new name,
rather than losing them all and gaining them back. The command exits with 0
whatever changed: it is for reading, not for failing a build. As with
`--since`, a ref that names no commit, and a project outside a git
repository, are errors. Only the head's unresolved imports and unreadable
files are reported, as `zenith graph` reports them; the base's are not the
working tree's to fix.

Each ref is read straight from git, without checking it out: nothing is
written, neither in the repository nor anywhere else, and Git LFS files are
read as their pointers, without fetching them. The files scanned at a ref
are those that git tracks at it, even with `respect_gitignore = false`. An
import that leads to what git ignores in the working tree, such as an
installed `node_modules`, is resolved there, so that it resolves at the ref
as it does now; the working tree's other untracked files are not part of
the ref. Rust crates are found from their `Cargo.toml` files at a ref,
without asking Cargo, which can only read the working tree. The files that
are the same at the ref as in the working tree reuse the parse cache, which
the ref never writes.

The project's root and configuration file are those at the same paths in the
ref; without that file, the ref's project uses the `zenith.toml` in its root,
or else the defaults. A configuration file outside the repository is used as
it is.

`zenith graph diff` takes a subcommand's place after `graph`, so to show a
directory called `diff`, write `zenith graph ./diff`.

## Exit codes

| Code | |
|---|---|
| 0 | Every contract of `error` severity is kept. |
| 1 | A contract of `error` severity is broken, or has stale baseline entries that are errors. |
| 2 | The configuration or the command line is invalid, or something went wrong. |

`zenith resolve` exits with 1 when the specifier doesn't resolve.
