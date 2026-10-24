# Output formats

`zenith check --format` picks how the results are shown.

## pretty

The default: each violation with an excerpt of the code that makes it, the
chains of imports it goes through, a hint on how to fix it, and the contract's
guidance; then a line for each contract and a summary.

```text
Contracts
  ✓ sdk-standalone   forbidden      kept
  ✗ web-layers       layers         broken (2 violations) · 1 suppressed
  ! ui-public-surface encapsulation broken (2 violations, warning only)

1 kept, 1 broken, 1 warning · 78 files, 127 imports
```

The summary line counts the kept and broken contracts, the broken contracts
of `warn` severity, and any stale baseline entries, then the files and imports
checked.

## compact

A line for each finding, `path:line:column: level[contract]: message; hint`,
where the hint says how to fix it, then the summary line. A finding about no
single import, such as an undeclared module or an exceeded limit, has no
`path:line:column: ` prefix.

## json

Everything the check found, for tools to read: each contract with its status,
violations, chains and counts, the unused suppressions, the imports that
couldn't be resolved, and with `--show-timings`, how long each step took. The
format has a version: fields may be added within a version, but none are
removed or change meaning.

### Fixes

Each finding of a violation says how to fix it: each chain, illegal import
and cycle breaker has a `fix`, and so does the violation of an undeclared
module or of a unit over a limit:

```json
"fix": {
  "action": "remove_import",
  "import": {
    "file": "web/src/components/upload.tsx",
    "line": 3,
    "column": 24,
    "specifier": "@/lib/images",
    "kind": "value"
  },
  "hint": "remove this import, or one further along the chain"
}
```

- `action` is what to do, one of the actions below.
- `import` is the import statement to change: its file, the line and column
  of its specifier, which are `null` if the file can't be read, the specifier
  as written, and the kind of import. It is `null` for a fix that is not
  about an import.
- `hint` says what to do in a line of plain language, as `pretty` and
  `compact` show it.

A fix never loosens a contract: every action but `declare_layer` changes
code, and `declare_layer` adds the module to the architecture. No fix raises
a limit.

| Action | Contract types | What to do |
| --- | --- | --- |
| `remove_import` | `forbidden`, `protected` | Remove the import, or for a chain, one further along it. |
| `move_module` | `independence`, `layers` | Move the imported code into a module that the importer may import, or remove the import. |
| `import_through_entry_point` | `encapsulation` | Import the module through one of its entry points. |
| `break_cycle` | `acyclic` | Remove the dependency, which breaks the cycle. |
| `declare_layer` | `layers` | Declare a module that an exhaustive contract doesn't know in `layers`. It has no `import`. |
| `split_module` | `limits` | Split the unit, or remove some of the imports that put it over its limit. It has no `import`. |

zenith doesn't apply fixes itself.

## github

A GitHub Actions workflow command for each finding, which GitHub shows as an
annotation on the line in a pull request, with the hint on how to fix it, then
the summary line. A finding about no single import, such as an undeclared
module or an exceeded limit, is an annotation on no line:

```yaml
- run: npx zenith check --format github
```

## sarif

A [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html)
log, which GitHub code scanning, GitLab and security dashboards read. Unlike
annotations, its alerts persist: GitHub shows them in the repository's
Security tab, and closes each one once a later upload no longer has it.

- Each checked contract is a rule, with its id, its name and type, its
  guidance as its help, and its severity as its level. Unused suppressions
  have a rule of their own, `zenith/unused-suppression`.
- Each finding, as `compact` lists it, is a result: an `error`, or a
  `warning` for a contract of `warn` severity, with its title as its
  message, and its [fix](#fixes), its `action` and `hint`, as
  `properties.fix`. It points at its import, with the rest of its chain of
  imports, or the other imports of a cycle breaker, as related locations, up
  to ten. Stale baseline entries and unmatched `ignore_imports` entries are
  results of their contract too. A finding without an import,
  such as an undeclared layer, points at its contract's table in
  `zenith.toml`; a unit over a limit does too, with the imports that make up
  its count as related locations. What suppressions and the baseline leave
  out is not a result.
- Each result has a fingerprint, `zenith/v1`, made from its contract and
  what it is about, such as the files of its chain, but not their lines. An
  alert stays the same alert when the code around its import moves.
- The imports that couldn't be resolved are notifications of the run rather
  than results.

Paths are relative to the root of the git repository that the project is in,
as code scanning reads them, even when the project's root is a directory
inside it. Inside a submodule, they are relative to the submodule, whose
repository code scanning reads. Outside a repository, they are relative to
the project root.

Upload the log with
[`github/codeql-action/upload-sarif`](https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/uploading-a-sarif-file-to-github).
zenith exits with 1 when a contract is broken, so let the upload run anyway:

```yaml
permissions:
  contents: read
  security-events: write
  actions: read # only needed in a private repository

steps:
  - uses: actions/checkout@v7
  - run: npx zenith check --format sarif > zenith.sarif
  - uses: github/codeql-action/upload-sarif@v4
    if: ${{ !cancelled() }}
    with:
      sarif_file: zenith.sarif
      category: zenith
```

Upload the log of a full check. Code scanning closes the alerts of a category
that an upload doesn't have, so the log of a check of some contracts, with
`--contract`, or of some files, with paths, closes the alerts of the others.

zenith's [GitHub Action](ci.md) installs zenith, checks and uploads in a
single step.
