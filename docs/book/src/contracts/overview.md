# Contracts

Contracts are tables under `[contracts.<id>]`. The id is how `--contract`, the
baseline and suppression comments refer to the contract: it may contain ASCII
letters, digits, `-` and `_`, and is kebab-case by convention.

<figure class="diagram">
{{#include ../diagrams/contracts.svg}}
<figcaption>The six contract types. Each diagram leads to its page.</figcaption>
</figure>

| Type | Rule |
|---|---|
| [`forbidden`](forbidden.md) | Some units must not import others, directly or through any chain of imports. |
| [`independence`](independence.md) | Units must not depend on one another. |
| [`layers`](layers.md) | Higher layers may import lower layers, never the reverse. |
| [`protected`](protected.md) | Only some importers may import a unit directly. |
| [`acyclic`](acyclic.md) | Units must not depend on each other in cycles. |
| [`encapsulation`](encapsulation.md) | A unit may only be imported through its entry points. |

## Common options

Every contract type takes these options:

<!-- ANCHOR: common-options -->

| Option | Default | |
|---|---|---|
| `type` | required | The [contract type](overview.md), such as `"layers"`. |
| `name` | the id | The title shown in the output. |
| `severity` | `"error"` | `"warn"` reports a broken contract without failing the check, for rolling a contract out gradually. |
| `exclude_kinds` | `[analysis] exclude_kinds` | The [import kinds](../concepts.md#imports) to leave out, e.g. `["type"]`. |
| `barrels` | `[analysis] barrels` | `"transparent"` sees an import of names from a [barrel](../concepts.md#barrels) as imports of the modules that define them, as bundlers load them; `"opaque"` as an import of the barrel. Not for `encapsulation`. |
| `ignore_imports` | `[]` | `"<selector> -> <selector>"` entries: the direct imports between them are left out of the contract. |
| `unmatched_ignore_imports` | `"error"` | Whether an `ignore_imports` entry that matches no import breaks the contract (`"error"`), is a warning (`"warn"`) or is fine (`"none"`). |
| `guidance` | none | Text shown under each violation, such as how to fix it. |

<!-- ANCHOR_END: common-options -->

An import between two files is left out of a contract when none of its
statements count: each statement may be of an excluded kind, suppressed with a
[comment](../suppressions.md), ignored, or in the [baseline](../baseline.md).
