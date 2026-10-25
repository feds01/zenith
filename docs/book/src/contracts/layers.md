# layers

Higher layers may import lower layers, never the reverse.

## Example

```toml
[contracts.package-tiers]
type = "layers"
layers = [
  "pkg:web | pkg:sdk | apps/* | services/*",
  "pkg:@acme/ui | pkg:@acme/forms",
  "pkg:@acme/client",
  "pkg:@acme/core",
  "pkg:@acme/types : pkg:@acme/config",
]
exclude_kinds = ["type"]
```

<figure class="diagram">
{{#include ../diagrams/layers.svg}}
<figcaption>Imports may only point down. <code>@acme/core</code> importing <code>@acme/forms</code> points up, and <code>web</code> importing <code>sdk</code> crosses an independent layer.</figcaption>
</figure>

The workspace packages are in five tiers, highest first, and each may import
any tier below it but nothing above. In the top tier, `web`, `sdk`, the apps
and the services may not import one another. At the bottom, `@acme/types` and
`@acme/config` may import each other. Type-only imports don't count.

## Writing layers

Each entry of `layers` is a string:

- `"a"` is a layer. Entries are selectors, so `"apps/*"` is one layer of every
  app, which may import each other.
- `"a | b"` is one layer of independent siblings, which may not import each
  other.
- `"a : b"` is one layer of siblings that may import each other. The `:` must
  have spaces around it, since `pkg:`, `npm:` and `node:` contain one too.
- `"(a)"` is an optional layer, which a container may not have.

An entry may also be a table, `{ layer = "a | b", closed = true }`. Layers
above a **closed** layer may not import the layers below it: they must go
through it.

```toml
layers = ["app", { layer = "service", closed = true }, "db"]
```

<figure class="diagram">
{{#include ../diagrams/layers-closed.svg}}
<figcaption><code>app</code> may import <code>service</code>, which may import <code>db</code>, but <code>app</code> may not import <code>db</code> past the closed layer.</figcaption>
</figure>

Layers may not overlap. The illegal dependencies are those from a lower layer
to a higher one, between independent siblings, and across a closed layer.
Each is searched for without passing through any other layer, so a chain
`low → mid → high` is reported as `low → mid` and `mid → high`.

## Containers

```toml
[contracts.domain-internals]
type = "layers"
containers = ["packages/api/src/domains/*"]
layers = ["hooks", "api", "types"]
```

<figure class="diagram">
{{#include ../diagrams/layers-containers.svg}}
<figcaption>Each domain is checked on its own. An import from one domain into another is outside the contract.</figcaption>
</figure>

Inside each domain, `hooks` may import `api` and `types`, `api` may import
`types`, and `types` may import neither. Each domain is checked on its own, so
imports between domains are outside this contract.

## Options

| Option | Default | |
|---|---|---|
| `layers` | required | Ordered from highest to lowest; see [Writing layers](#writing-layers). |
| `containers` | none | Selectors. If set, the layers are relative to each container, and each container is checked on its own. |
| `exhaustive` | `false` | Every child of every container must be declared as a layer. Needs `containers`. |
| `exhaustive_ignores` | `[]` | Names of children that `exhaustive` doesn't require, with or without their extension. |
| `allow_indirect_imports` | `false` | Only direct imports count. |
| `boundaries` | `[]` | Selectors of the modules that chains of imports may start or end at, but not pass through. See [boundaries](overview.md#boundaries). |

## Common options

Like every contract type, `layers` also takes:

{{#include overview.md:common-options}}
