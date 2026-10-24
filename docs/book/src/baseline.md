# The baseline

A codebase that adopts zenith usually breaks its contracts in many places at
first. The baseline records those violations, so `zenith check` only fails on
new ones, and the debt can only shrink.

```sh
zenith baseline
```

writes `zenith.baseline.toml`, which is meant to be committed. Its entries
are direct imports, sorted by contract, one a line, so changes to it are easy
to review:

```toml
version = 1

[contracts]
web-layers = [
    "web/src/lib/orders.ts -> web/src/domains/orders/api/client.ts",
]
```

- `zenith check` leaves each contract's entries out of it, as if they were
  `ignore_imports`.
- An entry whose import no longer exists is **stale**, and fails the check, so
  the baseline shrinks as violations are fixed. `zenith baseline --prune`
  removes the stale entries without adding any. With `[baseline] stale =
  "warn"`, stale entries are only warnings.
- Renaming or moving a file with baselined imports would make its entries
  stale, and its imports new violations. `zenith baseline --prune` follows
  the files that git sees renamed since the last commit, so their entries
  take the new names. `zenith check` doesn't, so prune the baseline before
  checking, for example in a pre-commit hook:

  ```sh
  zenith baseline --prune --tracked
  zenith check --tracked
  ```
- `zenith check --no-baseline` ignores the file.
- When a merge leaves git's conflict markers in the file, zenith reads the
  entries of both sides, and warns. The entries that either side fixed are
  then stale, so `zenith baseline --prune` resolves the conflict.

## How entries are found

For contracts that only check direct imports (`protected`, `encapsulation`,
and any with `allow_indirect_imports`), the entries are the offending imports.
For the others, zenith records the first import of each chain, the one made by
the importing unit's own code, then checks the contract again, until it is
kept. For `acyclic`, the entries are the imports of the cycle breakers. For
`limits`, they are the imports of the counted nodes over the limit, the last
ones by path, so the unit is left at its limit and any new importer (or
imported node) breaks it.

## Updating it

- `zenith baseline --contract <id>` updates only the entries of the given
  contracts, and keeps the others.
- Without `--contract`, the entries of contracts that no longer exist are
  dropped.
- When no entries are left, the file is removed.
- When the baseline gains entries, zenith warns, so new debt is reviewed.
