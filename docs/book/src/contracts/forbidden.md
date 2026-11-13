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

## Server components and client-only APIs

Next.js fails a build when a server component reaches a module without
`"use client"` that imports a client-only React API by name, such as
`useLayoutEffect`, often through a barrel that `export *`s a hook. A `to`
selector with [`names`](../selectors.md#the-table-form) counts only the imports
that take one of them, and `except` leaves the client components out of
`from`:

```toml
[contracts.server-components-client-apis]
type = "forbidden"
name = "Server components never reach client-only React APIs"
from = [{ path = "web/src/app", except = [{ path = "web/src/app", directive = "use client" }] }]
to = [
  { path = "npm:react", names = ["useState", "useEffect", "useLayoutEffect", "useReducer", "useRef", "useSyncExternalStore", "useTransition", "useImperativeHandle", "useInsertionEffect", "useDeferredValue", "useOptimistic", "useActionState", "createContext", "Component", "PureComponent"] },
  "npm:client-only",
]
boundaries = [{ path = "**", directive = "use client" }]
```

Chains stop at client components, which may use these APIs, and a chain is
reported only where it ends with an import that takes one of the names: a
server component may import `forwardRef` or `useId`. The chain points at the
statement that takes the name, such as `import { useLayoutEffect } from
"react"`, not at another import of `react` in the same file. A chain can still
pass through a module that `names` doesn't count an import of, on its way to
another `to` unit.

## Options

| Option | Default | |
|---|---|---|
| `from` | required | Selectors. |
| `to` | required | Selectors, which may be external packages and builtins, and may have [`names`](../selectors.md#the-table-form). |
| `allow_indirect_imports` | `false` | Only direct imports count. |
| `boundaries` | `[]` | Selectors of the modules that chains of imports may start or end at, but not pass through. See [boundaries](overview.md#boundaries). |

## Common options

Like every contract type, `forbidden` also takes:

{{#include overview.md:common-options}}
