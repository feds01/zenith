# Contracts

Contracts are tables under `[contracts.<id>]`. The id is how `--contract`, the
baseline and suppression comments refer to the contract: it may contain ASCII
letters, digits, `-` and `_`, and is kebab-case by convention.

<figure class="diagram">
{{#include ../diagrams/contracts.svg}}
<figcaption>The seven contract types. Each diagram leads to its page.</figcaption>
</figure>

| Type | Rule |
|---|---|
| [`forbidden`](forbidden.md) | Some units must not import others, directly or through any chain of imports. |
| [`independence`](independence.md) | Units must not depend on one another. |
| [`layers`](layers.md) | Higher layers may import lower layers, never the reverse. |
| [`protected`](protected.md) | Only some importers may import a unit directly. |
| [`acyclic`](acyclic.md) | Units must not depend on each other in cycles. |
| [`encapsulation`](encapsulation.md) | A unit may only be imported through its entry points. |
| [`limits`](limits.md) | A unit may only be imported by so many files, and import so many modules. |

## Common options

Every contract type takes these options:

<!-- ANCHOR: common-options -->

| Option | Default | |
|---|---|---|
| `type` | required | The [contract type](overview.md), such as `"layers"`. |
| `name` | the id | The title shown in the output. |
| `severity` | `"error"` | `"warn"` reports a broken contract without failing the check, for rolling a contract out gradually. |
| `exclude_kinds` | `[analysis] exclude_kinds` | The [import kinds](../concepts.md#imports) to leave out, e.g. `["type"]`. |
| `reexports` | `[analysis] reexports` | `"transparent"` sees an import of names from a [pass-through module](../concepts.md#pass-through modules) as imports of the modules that define them, as bundlers load them; `"opaque"` as an import of the pass-through module. Not for `encapsulation`. |
| `ignore_imports` | `[]` | `"<selector> -> <selector>"` entries: the direct imports between them are left out of the contract. |
| `unmatched_ignore_imports` | `"error"` | Whether an `ignore_imports` entry that matches no import breaks the contract (`"error"`), is a warning (`"warn"`) or is fine (`"none"`). |
| `guidance` | none | Text shown under each violation, such as how to fix it. |

<!-- ANCHOR_END: common-options -->

An import between two files is left out of a contract when none of its
statements count: each statement may be of an excluded kind, suppressed with a
[comment](../suppressions.md), ignored, or in the [baseline](../baseline.md).

## Boundaries

`forbidden`, `independence` and `layers` contracts follow chains of imports,
unless `allow_indirect_imports` is set. Their `boundaries` are selectors of
the modules that a chain may start or end at, but not pass through. A client
component may call a server action, but only a reference to it ships to the
browser, so the `"use server"` files are boundaries of a contract about what
client code bundles:

```toml
[contracts.client-safety]
type = "forbidden"
from = [{ path = "src/**", directive = "use client" }]
to = ["npm:sharp", "node:*"]
boundaries = [{ path = "src/**", directive = "use server" }]
```

A module that imports many others and is imported by them, such as a Rust
crate root that re-exports its modules, joins each of them to every other in
a chain through it. As a boundary, it still counts for its own imports, and
for those into it, but chains no longer pass through it:

```toml
boundaries = ["crates/*/src/lib.rs"]
```

Boundaries with `allow_indirect_imports = true` are a configuration error,
since direct imports pass through nothing.
