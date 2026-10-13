# encapsulation

Code outside a unit may only import it through its **entry points**, while
code inside it may import anything in it: the "public API" rule, with no deep
imports into a package or a feature's folder. import-linter has no equivalent.

## Example

```toml
[contracts.ui-public-surface]
type = "encapsulation"
modules = ["pkg:@acme/ui"]
entrypoints = ["index", "src/components/*/index"]
severity = "warn"
```

<figure class="diagram">
{{#include ../diagrams/encapsulation.svg}}
<figcaption>Outside <code>@acme/ui</code>, code may import its entry points. Importing <code>Button.tsx</code> directly is a deep import.</figcaption>
</figure>

Outside `@acme/ui`, code may only import the package's `index` and the
`index` of each component's directory. `web` importing
`@acme/ui/src/components/button/Button.tsx` is a deep import, reported as a
warning while the existing ones are migrated.

## Options

| Option | Default | |
|---|---|---|
| `modules` | required | Selectors of directory units. |
| `entrypoints` | `["index"]` | Globs relative to each unit's root, with or without an extension. They match files only: `public/**` exposes a directory. |
| `allowed_importers` | `[]` | Selectors of importers that may import anything, such as tests. |

## Common options

Like every contract type, `encapsulation` also takes:

{{#include overview.md:common-options}}
