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
where the hint says how to fix it, then the summary line.

## json

Everything the check found, for tools to read: each contract with its status,
violations, chains and counts, the unused suppressions, the imports that
couldn't be resolved, and with `--show-timings`, how long each step took. The
format has a version: fields may be added within a version, but none are
removed or change meaning.

### Fixes

Each finding of a violation says how to fix it: each chain, illegal import
and cycle breaker has a `fix`, and so does an undeclared module's violation:

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
code, and `declare_layer` adds the module to the architecture.

| Action | Contract types | What to do |
| --- | --- | --- |
| `remove_import` | `forbidden`, `protected` | Remove the import, or for a chain, one further along it. |
| `move_module` | `independence`, `layers` | Move the imported code into a module that the importer may import, or remove the import. |
| `import_through_entry_point` | `encapsulation` | Import the module through one of its entry points. |
| `break_cycle` | `acyclic` | Remove the dependency, which breaks the cycle. |
| `declare_layer` | `layers` | Declare a module that an exhaustive contract doesn't know in `layers`. It has no `import`. |

zenith doesn't apply fixes itself.

## github

A GitHub Actions workflow command for each finding, which GitHub shows as an
annotation on the line in a pull request, with the hint on how to fix it, then
the summary line:

```yaml
- run: npx zenith check --format github
```
