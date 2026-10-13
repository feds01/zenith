# acyclic

The `modules` units must not depend on each other in cycles. One unit depends
on another when any of its members imports a member of the other.

## Example

```toml
[contracts.packages-acyclic]
type = "acyclic"
modules = ["pkg:*"]
exclude_kinds = ["type"]
```

<figure class="diagram">
{{#include ../diagrams/acyclic.svg}}
<figcaption><code>@acme/client</code> and <code>@acme/core</code> depend on each other. Removing core's one import of client breaks the cycle, so zenith nominates it.</figcaption>
</figure>

The workspace packages must not depend on each other in cycles. If a file in
`@acme/core` imports `@acme/client`, and one in `@acme/client` imports
`@acme/core`, the report names the cheaper dependency to remove, such as
`@acme/core -> @acme/client (1 import)`. Type-only imports make no cycle at
runtime, so they don't count: `@acme/core`'s type import of `@acme/ui` would
otherwise close a second cycle.

## Cycle breakers

When there are cycles, zenith nominates the **cycle breakers**: the fewest
dependencies whose removal would make the units acyclic, found with grimp's
weighted heuristic, ties broken by name. Each breaker is shown with its number
of imports and a cycle through it.

## Options

| Option | Default | |
|---|---|---|
| `modules` | required | Selectors, e.g. `["pkg:*"]` or `["web/src/domains/*"]`. |
| `depth` | `0` | How many generations further to check: with `depth = 1`, the children of each directory unit are checked as units of their own too. |
| `skip` | `[]` | Selectors of units not to check the children of. |

## Common options

Like every contract type, `acyclic` also takes:

{{#include overview.md:common-options}}
