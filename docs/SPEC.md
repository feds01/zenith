# Zenith build spec

Status: **confirmed**. Version 1.1 of the spec, 2026-10-08. A complete prototype of
M0–M4 lives on the `feds01/full-implementation` branch, and lands on `main` one
reviewed PR at a time.

**Decisions** (confirmed 2026-10-08):

- The config format is TOML (`zenith.toml`), with a generated JSON Schema.
- The toolchain is a pinned nightly (`nightly-2026-08-07`), matching bracketlint.
- Scope: prototype M0–M4 first, then land it on `main` one reviewed PR at a time.
- Hosting: a private GitHub repository, `feds01/zenith`.

Places where implementation settled details the draft left open are marked
**(settled)**.

Zenith is an architecture linter for TypeScript and JavaScript codebases, with
first-class support for monorepos. You declare *contracts* about which parts of
a codebase may import which other parts, and `zenith check` fails when the code
breaks them. It is a single Rust binary.

The contract model is adapted from Python's
[import-linter](https://github.com/seddonym/import-linter). The graph
algorithms are ports of its engine, [grimp](https://github.com/python-grimp/grimp).
Both are re-shaped for how JS/TS code is organised: files and directories
instead of dotted modules, workspace packages, path aliases, `exports` maps,
type-only imports and barrel files.

The reference test-bed is a large private bun monorepo. It is not checked in;
synthetic fixtures that reproduce its patterns land with the UI tests
([§12](#12-testing)). The findings that shaped this spec are
summarised in [Appendix A](#appendix-a-findings-from-the-reference-monorepo).

---

## 1. Goals and non-goals

**Goals**

1. Declarative contracts that cover the common architecture rules: layers,
   forbidden dependencies, independence, protected modules, acyclicity and
   encapsulation.
2. Correct resolution on real monorepos, matching what TypeScript and the
   bundler see: per-package `tsconfig` `paths`, workspace packages, `exports`
   maps, `.js`→`.ts` remapping. It must work **without `node_modules`
   installed**, so it can run as a cheap CI job.
3. Adoptable on large, already-violating codebases: a **baseline ratchet**,
   **type-only exemptions**, inline suppressions and warn-level contracts.
4. Actionable output: every violation points at `file:line:col`, with the full
   import chain when the violation is indirect.
5. Fast: a full check of a ~6k-file monorepo, cold, in under 1.5 s on a laptop.
6. CI-friendly: stable exit codes, JSON output and GitHub annotations.

**Non-goals for v1** (candidates for later, see [§13](#13-roadmap))

- Symbol-level edges through barrel files (v1 edges are module-level).
- Custom contract plugins, an LSP or editor integration, watch mode, autofix.
- Checking `package.json` dependency hygiene (the territory of knip and sherif).
- Analysing non-JS languages, or framework-specific entry-point discovery.

---

## 2. Concepts

### 2.1 Project root

The project root is the directory containing `zenith.toml`. Zenith finds it by
searching upward from the working directory, or takes it from `--config <path>`.
All paths in config, output and the baseline are **POSIX paths relative to the
root**, on every OS.

### 2.2 The module tree

Zenith models the codebase as a tree of **nodes**:

| Node kind | Id example | Notes |
|---|---|---|
| Directory | `web/src/domains` | Exists for every ancestor of a file node. Directories have no imports of their own. |
| File | `web/src/domains/ai/api/client.ts` | Every scanned source file, plus every resolved import target inside the root, even if it isn't scanned (`.css`, `.svg`, `.json`, `.liquid`, a `dist/` file). Only source files have outgoing imports. |
| External package | `npm:react`, `npm:@tanstack/react-query` | Squashed to the package name, so `lodash/get` becomes `npm:lodash`. Leaf nodes outside the tree. |
| Builtin | `node:fs`, `bun:sqlite` | Node builtins are normalised to `node:` whether or not the source wrote the prefix. Subpaths are squashed (`node:fs/promises` becomes `node:fs`). |

A directory node stands for its whole subtree; this is the JS equivalent of a
Python package. There is no special "own module" for a directory: an
`index.ts` is an ordinary child file.

### 2.3 Workspace packages

Zenith discovers workspace packages from the root `package.json` `workspaces`
field (array or `{ packages }`) and from `pnpm-workspace.yaml`. Each one has a
**name** (from its `package.json`) and a **directory**. Names and directories
often differ (`packages/designSystem` is `@acme/design-system`), so contracts can refer
to packages by name with `pkg:` selectors ([§3](#3-selectors)).

### 2.4 Imports (edges)

An **import** is a directed edge from a file node to a file, external package
or builtin node, with one or more **details**. Each detail records the line,
column and span of the statement, the raw specifier, and its **kind**:

| Kind | Produced by |
|---|---|
| `value` | `import x from "s"`, `import "s"`, `export { a } from "s"`, `export * from "s"`, `export * as ns from "s"` |
| `type` | `import type …`, `export type …`, `import { type A, type B }` where *every* specifier is type-only, `export type * from`, `typeof import("s")` and other TS import types, `import type x = require("s")` |
| `dynamic` | `import("s")` with a string-literal or substitution-free template argument |
| `require` | `require("s")` with a literal argument, `import x = require("s")` |

Imports with non-literal specifiers are skipped and counted (`--verbose` lists
them). `vi.mock("s")` and `jest.mock("s")` are not imports in v1.

A pair `(importer, imported)` is present in a contract's view of the graph if
**at least one** of its details survives that contract's filters (excluded
kinds, suppressions, ignored imports, baseline).

### 2.5 Units

Contracts operate on **units**. A unit is a labelled set of member nodes.
A selector ([§3](#3-selectors)) resolves to a list of units:

- each matched **directory** becomes one unit, containing its whole subtree;
- each matched **file**, **external** or **builtin** node becomes a unit of one;
- `except` removes members from each unit;
- `directive` replaces the units with **one unit per member file** that has the
  directive ([§2.6](#26-directives)).

Two units **overlap** if they share a member. Contracts skip overlapping pairs,
as import-linter does, so `from = ["a"]`, `to = ["a/*"]` is legal and checks
nothing between `a` and itself.

### 2.6 Directives

A **directive** is a string-literal statement at the very top of a JS/TS file,
before any import or other code. Together they form the file's *directive
prologue*:

```ts
"use client";

import { useState } from "react";
```

Directives mark how a file runs. In React Server Components (Next.js and other
frameworks), `"use client"` makes a file the entry point of a client bundle, so
everything it imports, directly or not, ships to the browser. `"use server"`
marks server functions that client code may call. `"use strict"` is the
classic, older example.

Zenith records the directives of every scanned file. **There is no fixed list**:
every string in the prologue is recorded as written, so framework-specific ones
(e.g. Next.js's `"use cache"`) work too. A `directive` selector
([§3.3](#33-table-form)) matches one exactly. Directives let a contract target
a boundary that the directory layout doesn't show, such as "no `"use client"`
file may reach `node:*`" ([§4.2](#42-forbidden)).

---

## 3. Selectors

A selector is a string, or a table for the advanced forms.

### 3.1 String forms

| Form | Matches | Example |
|---|---|---|
| path glob | internal nodes (directories and files) | `web/src/domains/*`, `packages/ai/src/**/server.ts` |
| `pkg:<name-pattern>` | the directory of each matching workspace package | `pkg:@acme/*`, `pkg:web` |
| `npm:<name-pattern>` | external package nodes | `npm:sharp`, `npm:@aws-sdk/*` |
| `node:<glob>` | builtins (`node:*`, and `bun:*` via `bun:<glob>`) | `node:*`, `node:fs` |

`pkg:*` never matches the root package, which would contain everything.

**(settled)** Package, external and builtin names are matched with *name
patterns*, not path globs. In a name pattern, `*` matches any run of characters
**including `/`**, so `pkg:*` and `npm:*` match scoped names like `@acme/api`.
Otherwise they follow the glob syntax below: `?`, `{a,b}` alternation and `\`
escapes work.

**(settled)** When a path glob matches nested nodes (e.g. `web/**` matches both
`web/src` and `web/src/a.ts`), only the **outermost** matches become units,
since they already contain the inner ones.

### 3.2 Glob syntax

Zenith uses its own segment-wise glob dialect, so that Next.js and TanStack
file names need no escaping:

- `/` separates segments and cannot be escaped. A leading `./` is ignored.
- `*` matches any run of characters **within** one segment.
- `**` as a **whole segment** matches zero or more segments. A **trailing**
  `/**` matches one or more, so `a/**` means strictly inside `a`, as in gitignore.
- `?` matches one character; `{a,b}` is alternation; `\` escapes the next character.
- `[`, `]`, `(`, `)`, `$`, `@`, `+` are **literal**. For example,
  `web/src/app/(dashboard)/[id]/page.tsx` matches that file literally.
- **Extension-insensitive files:** a file also matches a pattern if the pattern
  matches its path with the JS/TS extension removed. So `src/types` matches
  both the directory `src/types` and the file `src/types.ts`. The stripped
  extensions are `.ts`, `.tsx`, `.mts`, `.cts`, `.d.ts`, `.js`, `.jsx`, `.mjs`
  and `.cjs`.

### 3.3 Table form

```toml
{ path = "web/src/**", directive = "use client" }   # "use client" files
{ path = "web/src/features", except = ["web/src/features/experimental"] }
```

`path` takes any string-form selector. `except` takes a list of selectors.
`directive` is an exact directive string from the file's prologue
([§2.6](#26-directives)).

**(settled)** A selector with a `directive` resolves to **one unit per matching
file**, rather than one per matched directory. "`use client` file X reaches
`npm:sharp`" is the meaningful unit of violation, and per-directory units would
report the same chain several times.

### 3.4 Unmatched selectors

A path or `pkg:` selector that matches nothing is a **configuration error**.
This catches typos and renames, as in import-linter. Layers can opt out with
the `(optional)` syntax ([§4.4](#44-layers)).

**(settled)**

- `npm:` and `node:` selectors may match nothing. It simply means nothing
  imports that package, so a `forbidden` contract with `to = ["npm:sharp"]` is
  kept.
- The check applies to the **base** selector, before `except` and `directive`
  filtering. `{ path = "apps", directive = "use client" }` in a project without
  client components is valid; it just selects nothing.
- Selectors in `ignore_imports` never error. An entry that matches nothing is
  handled by `unmatched_ignore_imports`.

---

## 4. Contracts

Contracts are tables under `[contracts.<id>]`. The id is required: it is
used by `--contract`, the baseline and suppression comments. Ids are
kebab-case and unique by construction, since they are TOML keys.

### 4.1 Shared options

| Option | Default | Meaning |
|---|---|---|
| `type` | required | `layers`, `forbidden`, `independence`, `protected`, `acyclic` or `encapsulation` |
| `name` | the id | Human-readable title for output |
| `severity` | `"error"` | `"warn"` reports the contract without failing the exit code, for gradual rollout |
| `exclude_kinds` | `[analysis] exclude_kinds` | Import kinds to ignore, e.g. `["type"]` |
| `ignore_imports` | `[]` | `"<selector> -> <selector>"` strings. Matching direct imports are removed before checking. |
| `unmatched_ignore_imports` | `"error"` | `"error"`, `"warn"` or `"none"` when an `ignore_imports` entry matches nothing |
| `guidance` | none | Text printed under the violations, e.g. how to fix them |

### 4.2 `forbidden`

The `from` units must not import the `to` units.

| Option | Default | |
|---|---|---|
| `from` | required | selectors |
| `to` | required | selectors; externals and builtins allowed |
| `allow_indirect_imports` | `false` | if `true`, only direct imports count |

By default the check is **transitive**: zenith finds every import chain from a
member of a `from` unit to a member of a `to` unit. The chain search is grimp's:
repeated bidirectional BFS, excluding each found chain's edges before searching
again.

```toml
[contracts.client-safety]
type = "forbidden"
name = "Client components never reach server-only code"
from = [{ path = "web/src/**", directive = "use client" }]
to = ["npm:sharp", "node:*", "npm:server-only"]
guidance = "Keep server-only modules out of client bundles."
```

No file under `web/src` that starts with `"use client"` may reach `sharp`, any
Node builtin, or `server-only`, directly or through any chain of imports. Each
client file is its own unit ([§2.6](#26-directives)), so a violation names the
client file and shows the whole chain, e.g. `web/src/components/upload.tsx` →
`web/src/lib/images.ts` → `npm:sharp`. Server components and route handlers
under `web/src` are unaffected, because they have no `"use client"` directive.

### 4.3 `independence`

The `modules` units must not depend on one another in either direction,
directly or indirectly. Chains that pass through a *third* unit are reported as
two separate violations rather than once (grimp's single independent layer).

| Option | Default |
|---|---|
| `modules` | required |
| `allow_indirect_imports` | `false` |

```toml
[contracts.features-independent]
type = "independence"
modules = ["web/src/features/*"]
ignore_imports = ["web/src/features/* -> web/src/features/shared"]
```

Each directory under `web/src/features` is a unit, and no feature may depend on
another. `features/checkout` importing `features/catalog/api.ts` is flagged. So
is an indirect route, such as `features/checkout` → `web/src/lib/cart.ts` →
`features/catalog`. `features/shared` also matches `features/*`, so without the
`ignore_imports` entry every feature that uses it would be flagged; the entry
exempts imports *into* `shared`, while `shared` importing a feature is still
flagged.

### 4.4 `layers`

Higher layers may import lower layers, never the reverse. This is grimp's
`find_illegal_dependencies_for_layers`, ported directly.

| Option | Default | |
|---|---|---|
| `layers` | required | ordered high to low; see the syntax below |
| `containers` | none | selectors. If set, `layers` entries are relative to each container, and each container is checked separately. |
| `exhaustive` | `false` | every child of every container must be declared as a layer (needs `containers`) |
| `exhaustive_ignores` | `[]` | child names exempt from the exhaustiveness check: name patterns matched against the base name with or without its JS/TS extension (`index` matches `index.ts`) |
| `allow_indirect_imports` | `false` | check only direct imports |

Each `layers` entry is a string:

- `"a"` is a single layer. Entries are selectors, so `"apps/*"` is allowed.
- `"a | b"` puts independent siblings in one layer: they may not import each other.
- `"a : b"` puts non-independent siblings in one layer: they may import each other.
- `"(a)"` makes the layer optional, so a container may lack it.
- Mixing `|` and `:` in one entry is an error.
- **(settled)** `:` only separates siblings when surrounded by whitespace
  (`"a : b"`), because it also appears in `pkg:`, `npm:` and `node:`
  selectors. `|` can be written with or without spaces.
- **(settled)** An entry that matches several modules (e.g. `"apps/*"`) forms
  **one** layer containing all of them; they may import each other. To make
  them independent of each other, add an `independence` contract.
- **(settled)** Layers may not overlap. Overlapping layers are a configuration
  error.

An entry can also be a table, `{ layer = "a | b", closed = true }`. A **closed**
layer may not be bypassed: layers above it may not import layers below it
(grimp's `closed`).

The illegal pairs are: lower → higher; sibling → sibling within an independent
layer; and higher → lower across a closed layer. For each illegal pair, zenith
searches for chains **excluding the members of every other layer**. A chain
`low → mid → high` is therefore reported as `low → mid` and `mid → high`, not
three times.

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

[contracts.domain-internals]
type = "layers"
containers = ["packages/api/src/domains/*"]
layers = ["hooks", "api", "types"]
```

`package-tiers` puts the workspace packages in five tiers, highest first. Each
tier may import any tier below it, and nothing above it:

- **Top:** `web`, `sdk`, every app and every service. `|` makes the four groups
  independent: an app may not import a service, `web` or `sdk`. Apps may import
  each other, because `apps/*` forms a single layer.
- **Then:** `@acme/ui` and `@acme/forms`, independent of each other.
- **Then:** `@acme/client`, then `@acme/core`.
- **Bottom:** `@acme/types` and `@acme/config`, which may import each other
  (`:`).

`@acme/core` importing `@acme/client` is flagged. `exclude_kinds = ["type"]`
exempts `import type` from it, since type-only imports are erased at runtime.

`domain-internals` applies the same idea inside each directory under
`packages/api/src/domains`, independently per domain: `hooks` (a directory) may
import `api` and `types`, `api` (e.g. `api.ts`) may import `types`, and `types`
may import neither. `domains/orders/types.ts` importing `domains/orders/api.ts`
is flagged. Importing `domains/billing/api.ts` is outside this contract,
because each container is checked on its own.

### 4.5 `protected`

Only `allowed_importers` (and the protected unit's own members) may **directly**
import a `modules` unit.

| Option | Default |
|---|---|
| `modules` | required |
| `allowed_importers` | required |

```toml
[contracts.agents-are-api-only]
type = "protected"
modules = ["web/src/agents"]
allowed_importers = ["web/src/app/api"]
```

Say `web/src/agents` holds LLM agents that read API keys and call paid
services, which should only run behind API routes. The contract lets only the
route handlers under `web/src/app/api`, and the agents themselves, import them
directly:

- **Flagged:** `web/src/components/chat.tsx` importing `@/agents/support`. A
  component that calls an agent directly would bundle it, or run it outside an
  API route.
- **Allowed:** `web/src/app/api/chat/route.ts` importing `@/agents/support`, an
  allowed importer; and `web/src/agents/support.ts` importing
  `web/src/agents/tools.ts`, two members of the protected unit.
- **Not checked:** `web/src/components/chat.tsx` importing `web/src/lib/run.ts`,
  which imports an agent. `protected` only checks *direct* imports, to keep
  the agents' list of importers short and deliberate. To forbid that
  reachability too, add a `forbidden` contract from `web/src/components` to
  `web/src/agents`.

### 4.6 `acyclic`

No dependency cycles among the `modules` units. Edges between units are
aggregated from direct imports between their members. With `depth`, zenith
then checks each directory unit's children as a new set of units, and so on
down `depth` more levels.

| Option | Default | |
|---|---|---|
| `modules` | required | e.g. `["pkg:*"]`, or `["web/src/domains/*"]` (import-linter's `ancestors = web/src/domains`) |
| `depth` | `0` | extra generations to drill into |
| `skip` | `[]` | units not drilled into |

When a cycle exists, zenith **nominates cycle breakers**: the smallest set of
unit-to-unit dependencies it can find whose removal makes the set acyclic. It
uses grimp's weighted Eades–Lin–Smyth heuristic with alphabetical tie-breaks,
so output is deterministic. The output names each breaker with its import
count and one example cycle through it.

```toml
[contracts.packages-acyclic]
type = "acyclic"
modules = ["pkg:*"]
exclude_kinds = ["type"]
```

Every workspace package is a unit, and the package graph must have no cycles.
If any file in `@acme/core` imports `@acme/client`, and any file in
`@acme/client` imports `@acme/core`, the contract breaks. The report names the
cheapest dependency to remove, e.g. `@acme/core -> @acme/client (1 import)`,
along with the cycle it closes. Type-only imports don't count, as they create
no runtime cycle.

### 4.7 `encapsulation` (new; not in import-linter)

Code outside a unit may only import that unit through its **entry points**.
Code inside the unit may import anything inside it. This is the "public API"
rule: no deep imports into a package or feature folder.

| Option | Default | |
|---|---|---|
| `modules` | required | directory units |
| `entrypoints` | `["index"]` | globs relative to each unit root. They are extension-insensitive, so `index` matches `index.ts`, `index.tsx` and so on. **(settled)** They match **files** only: to expose a whole directory, use `public/**`. |
| `allowed_importers` | `[]` | importers that may bypass the rule (e.g. tests) |

```toml
[contracts.ui-public-surface]
type = "encapsulation"
modules = ["pkg:@acme/ui"]
entrypoints = ["index", "src/components/*/index"]
severity = "warn"
```

Code outside `@acme/ui` may only import the package's root `index` and the
`index` of each component directory, e.g. `src/components/button/index.ts`.
`web` importing `@acme/ui/src/components/button/Button.tsx` is reported as a
deep import, while files inside `@acme/ui` may import each other freely.
`severity = "warn"` reports the violations without failing the check, for
example while existing deep imports are migrated.

---

## 5. Configuration file

`zenith.toml` lives at the project root. Unknown keys are errors
(`deny_unknown_fields`), with "did you mean" suggestions. A JSON Schema is
generated from the config types (`schemars`) and published as
`zenith.schema.json`, so Taplo and Even Better TOML can autocomplete with
`#:schema`.

```toml
#:schema https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json

[files]
include = ["**/*.{ts,tsx,mts,cts,js,jsx,mjs,cjs}"]   # default
exclude = ["experimental/**", "**/*.gen.ts"]       # added to the defaults below
respect_gitignore = true                           # default

[resolve]
tsconfig = "auto"          # "auto": nearest tsconfig.json per file; "off"; or a path
conditions = ["types", "source", "import", "module", "require", "node", "default"]   # default
main_fields = ["module", "main"]                   # default
alias = { "types" = "web/src/types" }         # bundler-only aliases, value relative to root
ignore_unresolved = ["/*", "virtual:*"]            # name patterns (§3.1) of specifiers not to report as unresolved

[analysis]
exclude_kinds = []         # default for every contract

[baseline]
path = "zenith.baseline.json"   # default
stale = "error"                 # "error" or "warn"

[contracts.<id>]
# see §4
```

The default excludes are always applied: `node_modules`, `.git`, `.yarn`,
`coverage`, `.next`, `.nuxt`, `.output`, `.svelte-kit`, `.turbo`, `.vercel`,
`.cache`, `.wxt`, `.mastra` and `storybook-static`, wherever they appear, and
Yarn's `.pnp.cjs` and `.pnp.loader.mjs`. `dist`, `build` and `out` are excluded
only directly inside a package (next to a `package.json`) and when they aren't
packages themselves, since elsewhere, as in `src/build/`, they may be source
code.

---

## 6. Resolution

Zenith resolves specifiers with
[`oxc_resolver`](https://crates.io/crates/oxc_resolver), a Rust port of
webpack's enhanced-resolve, through `resolve_file`. With
`TsconfigDiscovery::Auto`, every file uses its nearest `tsconfig.json`, with
`extends` and project references honoured. Defaults:

- `extensions`: `.ts .tsx .d.ts .mts .cts .js .jsx .mjs .cjs .json`
- `extension_alias`, in the order TypeScript tries them:
  `.js → [.ts, .tsx, .d.ts, .js, .jsx]`, `.jsx → [.tsx, .ts, .d.ts, .jsx, .js]`,
  `.mjs → [.mts, .d.mts, .mjs]`, `.cjs → [.cts, .d.cts, .cjs]`
- `builtin_modules: true`, `symlinks: true` (realpath, so bun and pnpm links
  land on workspace sources), `node_path: false` (`NODE_PATH` is ignored)
- **Workspace fallback (settled):** when normal resolution of a bare
  specifier fails and its package name is a workspace package, zenith resolves
  it directly against the package directory. If the package has an `exports`
  map, zenith resolves through it (subpaths, condition order, `*` patterns,
  `null` exclusions; see `zenith_resolve/src/exports.rs`). A subpath the map
  does not export is unresolved, matching TypeScript and Node. Without
  `exports`, the subpath is a path inside the package. So workspace imports
  resolve identically with and without `node_modules`.
- A file whose `tsconfig.json` can't be loaded (because it extends one in a
  package that isn't installed, say) is resolved without it. A non-relative
  specifier that then fails is unresolved, with the `tsconfig.json` error as
  the reason.
- **Default conditions (settled):** `types`, `source`, `import`, `module`,
  `require`, `node`, `default`. They are matched in the order the package
  lists them. `types` is included so that internal TS packages resolve to
  source files, not build output.

Each resolution is then classified:

| Result | Becomes |
|---|---|
| path inside the root, not under `node_modules` | internal file node |
| path under `node_modules`, or outside the root | `npm:<package name taken from the specifier>` (for a specifier that isn't a package name, from the path after its last `node_modules`); unresolved if there is none |
| builtin error, or a `node:` or `bun:` specifier | `node:<name>` (or `bun:<name>`) |
| a bare specifier that fails to resolve | `npm:<package name>`, flagged *unresolved*. This keeps `npm:` contracts working in CI without `node_modules`. |
| any other specifier that fails: relative or absolute, another URL scheme (`virtual:`), a path alias that can't be a package name (`@/x`, `~/x`), or a `[resolve] alias` match | unresolved, not in the graph, reported as a warning (unless it matches `ignore_unresolved`) |

`zenith check` prints one summary line for unresolved imports, and
`--show-unresolved` lists them.

---

## 7. Baseline (ratchet)

Large codebases start out broken. The baseline records the existing
violations, so a check fails only on **new** ones, and the debt can only shrink.

- `zenith baseline` runs every contract (or the `--contract` ones) and writes
  `zenith.baseline.json`. Entries are concrete **direct imports**, grouped by
  contract and sorted, so diffs are reviewable:
  ```json
  {
    "version": 1,
    "contracts": {
      "web-layers": [
        "web/src/lib/orders.ts -> web/src/domains/orders/api/client.ts"
      ]
    }
  }
  ```
- `zenith check` treats each contract's entries as extra `ignore_imports`.
  An entry whose import no longer exists is **stale** and fails the check
  (`[baseline] stale = "warn"` relaxes this). Stale failures make the baseline
  shrink as violations are fixed; `zenith baseline --prune` removes stale
  entries without adding new ones.
- **Generating the entries.** For direct-import contracts (`protected`,
  `encapsulation`, and any contract with `allow_indirect_imports`), the entries
  are the offending imports. For chain contracts, zenith repeats: take the
  **first link** of every remaining chain (the import made by the downstream
  unit's own code), add it, and re-check until the contract is kept. For
  `acyclic`, the entries are the nominated cycle breakers.
- `--no-baseline` ignores the file.
- **(settled)** `zenith baseline` keeps the entries of contracts that weren't
  selected with `--contract`. Without `--contract`, it drops entries of
  contracts that no longer exist. When no entries remain, it deletes the file.
  It warns when the baseline grows, so new debt is reviewed.

---

## 8. Inline suppressions

```ts
// zenith-ignore web-layers: legacy, tracked in ENG-123
import { x } from "@/domains/orders";

import { y } from "@/app/(dashboard)/types"; // zenith-ignore web-layers
```

- Syntax: `zenith-ignore <id>[ <id>…][: <reason>]`, in a line or block comment.
- A suppression applies to imports that start on the same line (trailing
  comment) or on the next line. It removes those import details from the
  named contracts only.
- An unused suppression is a warning. **(settled)** It is unused if it names
  no contract, names an unknown contract, or none of the contracts it names
  (among those being checked) had an import starting on its target line.

---

## 9. CLI

```
global: [--config <path>] [--root <dir>] [--color auto|always|never] [-v…]

zenith check [--contract <id>]… [--format pretty|compact|json|github]
             [--no-baseline] [--show-unresolved] [--show-timings]
zenith baseline [--contract <id>]… [--prune]
zenith graph [<selector>] [--depth <n>] [--format tree|dot|json] [--exclude-kind <kind>]…
zenith why <from-selector> <to-selector> [--exclude-kind <kind>]… [--limit <n>]
zenith resolve <file> <specifier>                                # how one specifier resolves, and why
zenith init [--force]                                            # write a starter zenith.toml from the discovered packages
zenith version
zenith schema                                                    # (hidden) print the JSON Schema of zenith.toml
```

**Exit codes:**

| Code | Meaning |
|---|---|
| 0 | every error-severity contract kept |
| 1 | at least one error-severity contract broken, or a stale baseline |
| 2 | configuration, usage or internal error |

`zenith resolve` exits 1 when the specifier does not resolve. The summary line
reads `<kept> kept, <broken> broken[, <n> warnings][ · <n> stale baseline entries] · <files> files, <imports> imports`,
where warnings are broken contracts with `severity = "warn"`.

`--root` points zenith at a project whose config lives elsewhere (used for
corpus runs). `-v` shows info logs, `-vv` debug logs (all on stderr).

The CLI mirrors teamsearch and bracketlint: clap derive, argfile expansion,
`wild` globbing on Windows, a crash handler, and version info from `build.rs`.
**(settled)** An `@path` argument is only expanded as an argfile when `path` is
a file, so arguments like `@scope/pkg` pass through unchanged.

---

## 10. Output

- **`pretty`** (the default): one block per broken contract.
  - A header with the contract id, type and severity.
  - Violations grouped by `downstream unit → upstream unit`.
  - Each chain rendered link by link as `file:line → file`, with
    import-linter's `&` notation for multiple heads and tails.
  - An `annotate-snippets` excerpt of the first link, with the guidance below
    it.
  - A final summary line, e.g. `9 kept, 2 broken, 1 warning · 1,234 files,
    5,678 imports` (see [§9](#9-cli)).
- **`compact`**: one line per violation, `path:line:col: <contract-id>: message`.
- **`json`**: the full report: contracts, status, findings, chains, warnings and
  timings. Its schema is versioned.
- **`github`**: `::error file=…,line=…,col=…,title=zenith(<id>)::…` workflow
  commands, plus the summary.

Colour is on when stdout is a TTY and `NO_COLOR` is unset, unless overridden
with `--color`.

---

## 11. Architecture

### 11.1 Pipeline

```
load config ─► discover workspace packages ─► walk files (ignore, parallel)
   ─► parse + extract imports (oxc_parser, rayon, per-thread allocator)
   ─► resolve specifiers (one shared oxc_resolver, rayon)
   ─► build Graph (single-threaded merge)
   ─► resolve selectors into units, per contract
   ─► check contracts (rayon; the graph is immutable)
   ─► apply baseline and suppressions ─► render ─► exit code
```

Contracts never mutate or clone the graph, unlike import-linter, which
deep-copies it per contract. Each contract builds a **view**: an
excluded-imports set made from its `ignore_imports`, baseline entries,
suppressions and kind filters. grimp's pathfinding already accepts excluded
imports and excluded modules, so this needs no new algorithms.

### 11.2 Crates

| Crate | Responsibility |
|---|---|
| `zenith` | The binary: `cli.rs`, `commands/*`, `crash.rs`, `version.rs`, `build.rs` |
| `zenith_config` | Config types (serde + schemars), discovery, TOML parsing, validation into typed contract definitions |
| `zenith_workspace` | File walker (adapted from the ruff-derived resolver in teamsearch and bracketlint), workspace package discovery |
| `zenith_parse` | oxc-based extraction of imports, directives and suppression comments |
| `zenith_resolve` | `oxc_resolver` setup and resolution classification, plus `exports`-aware workspace resolution |
| `zenith_graph` | The module tree and import graph: interned tokens, hierarchy, edges with details, pathfinding, chains, layers, cycle breakers (a grimp port). No IO. |
| `zenith_index` | Orchestrates walk, parse and resolve into a `Graph` |
| `zenith_contracts` | Selectors and units, per-contract views, the six contract types, the findings model, baseline, suppressions |
| `zenith_reporting` | Renderers (`pretty`, `compact`, `json`, `github`) using `annotate-snippets` |
| `zenith_utils` | Logging, output streams, highlighting, the glob dialect and name patterns, printing helpers, tree writing, line index, byte ranges, timers, path helpers (mostly from bracketlint) |

**(settled)** The UI snapshot harness lives at `crates/zenith/tests/ui.rs` (a
`harness = false` integration test using `libtest-mimic`), not in a separate
`tests` crate. That way it can run the real binary and check exit codes.

### 11.3 Key dependencies

`oxc_parser`, `oxc_ast`, `oxc_ast_visit`, `oxc_allocator`, `oxc_span`,
`oxc_resolver`, `rayon`, `ignore`, `clap`, `serde`, `toml`, `serde_json`,
`schemars`, `annotate-snippets`, `rustc-hash`, `indexmap`, `anyhow`/`thiserror`,
`log`, `wild`, `argfile`, `yaml-rust2` (for `pnpm-workspace.yaml`); for tests, `libtest-mimic` and `pretty_assertions`.

---

## 12. Testing

- **Unit tests** in each crate: the glob dialect, selector resolution, the
  grimp algorithm tests ported from grimp (layers permutations, shortest chains,
  cycle breakers), and import extraction for every syntax form in §2.4.
- **UI tests** in `tests/cases/<group>/<case>/`. Each case is a project with:
  - an `args` file, with one invocation per line (default `check`);
  - an optional `fixture` file, naming a project in `tests/fixtures/` that is
    copied first;
  - one `output.snap` recording each invocation's exit code, stdout and stderr.

  Each case is copied to a temporary directory outside the repository and runs
  as its own test through `libtest-mimic`. `just update-snapshots`
  (`REGENERATE_OUTPUT=1`) re-records them. The harness and its `just` recipes
  land with the CLI PR.
- **The `acme` fixture**: a synthetic monorepo in `tests/fixtures/acme`,
  modelled on the reference test-bed. It has:
  - bun workspaces, with package names that differ from their directories;
  - `@/*` and catch-all `*` tsconfig paths;
  - deep `@acme/x/src/...` imports, namespace barrels, `"use client"` files;
  - type-only imports, Next.js route groups and `[param]` directories;
  - `.js`→`.ts` imports and asset imports.
- **Corpus run**: `just corpus <path> <config>` runs a release build against a
  local checkout, with a config kept in the gitignored `corpus/` directory. Use
  it for performance and sanity checks on the real test-bed.
- CI runs the test suite on Linux, macOS and Windows, because path handling
  has to hold up on all three.
- **Benchmarks** live in `crates/zenith_benchmark`, following bracketlint's
  `bl_benchmark`:
  - They are written with [divan](https://github.com/nvzqz/divan) through
    `codspeed-divan-compat`, and run locally with `just bench`. Extra arguments
    filter, e.g. `just bench parse`. Allocations are counted with divan's
    `AllocProfiler`.
  - **Inputs:** the `acme` fixture, plus a synthetic monorepo generated
    deterministically at several sizes (e.g. 1k, 10k and 50k files). The
    synthetic repo has workspace packages, path aliases, barrels and a
    realistic import density, so that scaling is measured without committing a
    large fixture.
  - **What is measured:**
    - parsing and import extraction, per file (bytes/s);
    - resolution;
    - building the index (walk, parse, resolve, graph);
    - each contract type on the synthetic graph;
    - a full `check`.
  - **CI:** a CodSpeed workflow measures every commit on `main` as the
    baseline, and PRs that touch benchmarked crates, in simulation and memory
    modes, as in bracketlint. Enabling CodSpeed on a private repository may
    need a paid plan; `just bench` works regardless.
  - **When it lands:** the crate arrives with the parse PR, and each later
    crate PR adds benchmarks for its hot paths.
- **Measured on the prototype:**

  | Run | Files | Imports | Contracts | Time |
  |---|---|---|---|---|
  | The reference monorepo, release build | ~6k | ~30k | 15 | ~0.2 s total (walk 50 ms, parse and resolve 95 ms, graph 14 ms) |

  A reachability pre-filter cut `forbidden` checks with many sources from
  820 ms to 32 ms on that run.

---

## 13. Roadmap

| Milestone | Scope |
|---|---|
| **M0** ✅ | Repository scaffold, toolchain, CI, release pipeline, this spec |
| **M1** | Config, workspace discovery, walker, parse, resolve, graph; `zenith graph`, `why` and `resolve` as debugging tools |
| **M2** | `forbidden`, `protected`, `independence` and `layers` contracts; `pretty` and `compact` output |
| **M3** | `acyclic` and `encapsulation` |
| **M4** | Baseline, suppressions, `json` and `github` output, `severity`, `init` |
| **M5** | Docs site, JSON schema publishing, and more distribution channels (an npm package, Homebrew) |
| later | A `"use server"` boundary rule; `vi.mock` specifiers as optional edges; an incremental cache; barrel-precise (symbol-level) edges through pure re-export files; a `package.json` declared-dependency check; an outgoing allowlist contract; an LSP |

---

## Appendix A. Findings from the reference monorepo

- **No boundary tooling, and drift everywhere.** The main app's top-level
  directories form one dependency cycle, and so do several shared packages.
  This is what makes the **baseline ratchet** essential.
- **Teams already hand-roll zenith.** Boundary rules exist as bespoke tests that
  walk imports from entry files, assert that some modules are never reached,
  and keep a list of known violations that fails on new *and* stale entries.
  These map directly to transitive `forbidden`, `layers` and the baseline.
- **Type-only imports number in the thousands**, and many existing violations
  are type-only. This is why `exclude_kinds` is per-contract.
- **No `exports` maps, `main: index.ts`, and thousands of deep `@pkg/src/...`
  imports.** This motivates `encapsulation`, `exports`-aware workspace
  resolution, and resolving without `node_modules`.
- **Path aliases are per package**, including a catch-all `*` → `./src/*` that
  shadows an npm package of the same name. That needs per-file tsconfig
  discovery and `[resolve] alias` for bundler-only aliases.
- **Hundreds of pure re-export barrels.** Module-level chains through barrels
  over-report reachability compared with what bundlers tree-shake. v1 accepts
  this, like import-linter. Barrel-precise edges are the headline v2 feature.
- **`"use client"` in a large share of the app's files.** This motivates the
  `directive` selector.
- **Next.js `(group)` and `[param]` directories, and TanStack `$id` files.**
  This motivates the glob dialect with literal brackets and parentheses.
- **Copies of the repo in `.worktrees/` inside the root.** gitignore must be
  respected by default.
