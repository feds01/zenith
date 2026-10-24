# limits

Each `modules` unit may be imported by at most `max_fan_in` files outside
it, and import at most `max_fan_out` files and packages of the project
outside it. A module that everything imports, or that imports everything, is
hard to change without touching much else.

## Example

```toml
[contracts.small-surface]
type = "limits"
modules = ["web/src/*"]
max_fan_in = 20
max_fan_out = 12
```

<figure class="diagram">
{{#include ../diagrams/limits.svg}}
<figcaption>With <code>max_fan_in = 2</code>, a third file importing <code>web/src/lib</code> breaks the contract. <code>app/main.ts</code> counts once, though it imports two of its files.</figcaption>
</figure>

Each directory of `web/src` is checked on its own:

- **Fan-in** is the number of files outside the unit that import it. A file
  counts once, however many of the unit's files it imports. A unit may be an
  external package: `modules = ["npm:*"]` limits how many files import each.
- **Fan-out** is the number of files outside the unit that it imports, and
  of packages that it imports as a whole, such as a Go package. Each counts
  once, however many of the unit's files import it.
- **Not counted:** imports between the unit's own files, imports of external
  packages such as `npm:react` and of builtins such as `node:fs`, and the
  imports that the contract leaves out: those of its `exclude_kinds`, its
  `ignore_imports`, suppressed ones, and those in the baseline. Only direct
  imports count. To restrict who may import an external package, use a
  [`protected`](protected.md) contract.

A unit over a limit is a violation that names the count and the limit, and
lists the imports that make up the count. A unit over both limits is two
violations.

## Options

| Option | Default | |
|---|---|---|
| `modules` | required | Selectors of the limited units. |
| `max_fan_in` | none | The most files outside each unit that may import it. |
| `max_fan_out` | none | The most files and packages of the project outside each unit that it may import. |

At least one of `max_fan_in` and `max_fan_out` is required.

## Common options

Like every contract type, `limits` also takes:

{{#include overview.md:common-options}}
