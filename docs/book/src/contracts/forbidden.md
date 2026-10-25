# forbidden

The `from` units must not import the `to` units, directly or through any chain
of imports.

## Example

```toml
[contracts.client-safety]
type = "forbidden"
name = "Client components never reach server-only code"
from = [{ path = "web/src/**", directive = "use client" }]
to = ["npm:sharp", "node:*", "npm:server-only"]
guidance = "Keep server-only modules out of client bundles."
```

<figure class="diagram">
{{#include ../diagrams/forbidden.svg}}
<figcaption><code>upload.tsx</code> reaches <code>npm:sharp</code> through <code>lib/images.ts</code>: one violation, shown with its chain.</figcaption>
</figure>

No file under `web/src` that starts with `"use client"` may reach `sharp`, a
Node builtin or `server-only`. Each client file is a unit of its own, so a
violation names it and shows the chain, such as
`web/src/components/upload.tsx` → `web/src/lib/images.ts` → `npm:sharp`.
Server components and route handlers are unaffected, since they have no
`"use client"`.

Each pair of units is one violation, with every chain between them, shortest
first: as in import-linter, once a chain is found its imports are set aside,
and the search runs again until no chain is left.

## Options

| Option | Default | |
|---|---|---|
| `from` | required | Selectors. |
| `to` | required | Selectors, which may be external packages and builtins. |
| `allow_indirect_imports` | `false` | Only direct imports count. |
| `boundaries` | `[]` | Selectors of the modules that chains of imports may start or end at, but not pass through. See [boundaries](overview.md#boundaries). |

## Common options

Like every contract type, `forbidden` also takes:

{{#include overview.md:common-options}}
