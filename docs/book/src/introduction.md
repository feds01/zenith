<div class="logo" aria-hidden="true">
{{#include ../../logo/zenith.svg}}
</div>

# zenith

zenith is an architecture linter for TypeScript and JavaScript monorepos. It
brings Python's [import-linter](https://import-linter.readthedocs.io/) to
them: the contract model is import-linter's, and the graph algorithms are
ports of its engine, [grimp](https://github.com/python-grimp/grimp), adapted
to how JavaScript projects are organised. It is written in Rust, so a check of
a monorepo with thousands of files takes well under a second.

## How it works

You write down how the parts of your codebase may depend on each other, as
**contracts** in a `zenith.toml`. zenith finds the project's source files and
workspace packages, parses their imports, and resolves each one to a file, a
package or a builtin: together, they make the project's **import graph**. It
then checks every contract against the graph, and reports each import, or
chain of imports, that breaks one.

<figure class="diagram">
{{#include diagrams/how-it-works.svg}}
<figcaption>zenith builds the import graph once, checks every contract against it, and reports the violations.</figcaption>
</figure>

## An example

Two contracts: the first puts the app's code in layers, so that lower layers
never import higher ones, and the second keeps server-only code out of client
bundles:

```toml
[contracts.web-layers]
type = "layers"
layers = ["web/src/app", "web/src/components", "web/src/lib"]
guidance = "Lower layers must not import higher ones."

[contracts.client-safety]
type = "forbidden"
from = [{ path = "web/src/**", directive = "use client" }]
to = ["npm:sharp", "node:*", "npm:server-only"]
guidance = "Keep server-only modules out of client bundles."
```

`zenith check` finds a component that imports from the app layer above it, and
a client component that reaches `sharp` through another module. It shows each
violation with the import, or the chain of imports, that makes it, then a line
per contract:

```text
error[web-layers]: `web/src/components` must not import `web/src/app`
 --> web/src/components/sidebar.tsx:3:32
  |
3 | import { DASHBOARD_TABS } from "@/app/tabs";
  |                                ^^^^^^^^^^^^ imports `web/src/app/tabs.ts`
  |
  = help: Lower layers must not import higher ones.

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

Contracts
  ✗ web-layers     layers     broken (1 violation)
  ✗ client-safety  forbidden  broken (1 violation)

0 kept, 2 broken · 412 files, 968 imports
```

## What it does

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
