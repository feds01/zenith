# independence

The `modules` units must not depend on one another, in either direction,
directly or through other modules.

| Option | Default | |
|---|---|---|
| `modules` | required | Selectors. The units may not overlap. |
| `allow_indirect_imports` | `false` | Only direct imports count. |

```toml
[contracts.features-independent]
type = "independence"
modules = ["web/src/features/*"]
ignore_imports = ["web/src/features/* -> web/src/features/shared"]
```

Each directory under `web/src/features` is a unit, and no feature may depend
on another: `features/checkout` importing `features/catalog/api.ts` breaks the
contract, and so does an indirect route such as `features/checkout` →
`web/src/lib/cart.ts` → `features/catalog`.

`features/shared` matches `features/*` too, so every feature that uses it
would be flagged. The `ignore_imports` entry exempts imports *into* `shared`,
while `shared` importing a feature is still flagged.

A chain through a third unit is reported as two violations, one on each side
of it.
