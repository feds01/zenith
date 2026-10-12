# Output formats

`zenith check --format` picks how the results are shown.

## pretty

The default: each violation with an excerpt of the code that makes it, the
chains of imports it goes through, and the contract's guidance; then a line
for each contract and a summary.

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

A line for each finding, `path:line:column: level[contract]: message`, then
the summary line.

## json

Everything the check found, for tools to read: each contract with its status,
violations, chains and counts, the unused suppressions, the imports that
couldn't be resolved, and with `--show-timings`, how long each step took. The
format has a version: fields may be added within a version, but none are
removed or change meaning.

## github

A GitHub Actions workflow command for each finding, which GitHub shows as an
annotation on the line in a pull request, then the summary line:

```yaml
- run: npx zenith check --format github
```
