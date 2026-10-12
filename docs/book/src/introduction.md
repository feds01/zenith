# zenith

zenith is an architecture linter for TypeScript and JavaScript monorepos. You
write down how the parts of your codebase may depend on each other, as
**contracts** in a `zenith.toml`, and `zenith check` tells you where the code
breaks them.

```toml
[contracts.client-safety]
type = "forbidden"
from = [{ path = "web/src/**", directive = "use client" }]
to = ["npm:sharp", "node:*", "npm:server-only"]
guidance = "Keep server-only modules out of client bundles."
```

```text
error[client-safety]: `web/src/components/upload.tsx` must not import `npm:sharp`
 --> web/src/components/upload.tsx:3:24
  |
3 | import { resize } from "@/lib/images";
  |                        ^^^^^^^^^^^^^^ starts a chain of 2 imports
  |
  = note: import chain:
            web/src/components/upload.tsx:3
            -> web/src/lib/images.ts:1
            -> npm:sharp
  = help: Keep server-only modules out of client bundles.
```

It is inspired by Python's [import-linter](https://import-linter.readthedocs.io/)
and its graph library, grimp, whose algorithms it ports, and is written in
Rust, so a check of a monorepo with thousands of files takes well under a
second.

What it does:

- **Six contract types:** [`forbidden`](contracts/forbidden.md),
  [`independence`](contracts/independence.md), [`layers`](contracts/layers.md),
  [`protected`](contracts/protected.md), [`acyclic`](contracts/acyclic.md) and
  [`encapsulation`](contracts/encapsulation.md).
- **Monorepo aware:** workspace packages are found from `package.json` and
  `pnpm-workspace.yaml`, and can be named in contracts with `pkg:` selectors.
  Imports between them resolve as TypeScript and Node do, with or without
  `node_modules` installed.
- **Built for adoption:** a [baseline](baseline.md) records the violations a
  codebase already has, so only new ones fail, and
  [suppressions](suppressions.md) exempt single imports.
- **Made for CI:** a stable exit code, and `compact`, `json` and `github`
  [output formats](output.md).
