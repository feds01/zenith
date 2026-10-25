# independence

The `modules` units must not depend on one another, in either direction,
directly or through other modules.

## Example

```toml
[contracts.features-independent]
type = "independence"
modules = ["web/src/features/*"]
ignore_imports = ["web/src/features/* -> web/src/features/shared"]
```

<figure class="diagram">
{{#include ../diagrams/independence.svg}}
<figcaption>A feature may not import another, directly or through a module outside them. Imports into <code>shared</code> are ignored.</figcaption>
</figure>

Each directory under `web/src/features` is a unit, and no feature may depend
on another: `features/checkout` importing `features/catalog/api.ts` breaks the
contract, and so does an indirect route such as `features/checkout` →
`web/src/lib/cart.ts` → `features/catalog`.

`features/shared` matches `features/*` too, so every feature that uses it
would be flagged. The `ignore_imports` entry exempts imports *into* `shared`,
while `shared` importing a feature is still flagged.

A chain through a third unit is reported as two violations, one on each side
of it.

## Options

| Option | Default | |
|---|---|---|
| `modules` | required | Selectors. The units may not overlap. |
| `allow_indirect_imports` | `false` | Only direct imports count. |
| `boundaries` | `[]` | Selectors of the modules that chains of imports may start or end at, but not pass through. See [boundaries](overview.md#boundaries). |

## Common options

Like every contract type, `independence` also takes:

{{#include overview.md:common-options}}
