# Expected results for the acme fixture

These are the results `zenith check` should produce for this fixture with
[`zenith.toml`](zenith.toml), under these assumptions:

- `node_modules` is **not** installed.
- `.gitignore` is respected, so `.worktrees/` is never scanned.
- Specifiers resolve the way TypeScript and the bundler would with
  `node_modules` installed. That includes the `@acme/ui` `exports` map.
- Edges are module-level, as in spec v1. Barrels (`export *`) pass
  reachability through to everything they re-export.

Paths are POSIX paths relative to this directory, and `file:line` is 1-based.
Every static `import` and `export … from` statement starts at **column 1**.
[Columns](#columns-of-expression-imports) lists the column of each
expression-level import.

**Totals:** 78 source files are scanned. They contain 128 literal import
details and 1 non-literal `import(name)`, which is skipped.

## Summary

| Contract | Type | Severity | Status | Findings |
|---|---|---|---|---|
| `web-layers` | layers | error | broken | 2 direct imports |
| `package-tiers` | layers | error | broken | 1 direct import |
| `packages-acyclic` | acyclic | error | broken | 1 cycle, 1 breaker |
| `client-safety` | forbidden | error | broken | 3 chains |
| `ai-public-surface` | encapsulation | **warn** | broken (warning) | 2 deep imports |
| `strategies-independent` | independence | error | broken | 1 direct import |
| `mastra-protected` | protected | error | broken | 1 direct import |
| `api-domain-layers` | layers | error | broken | 1 direct import |
| `legacy-freeze` | forbidden | error | broken | 1 chain (one link) |
| `sdk-standalone` | forbidden | error | **kept** | 0 |

The expected exit code is **1**: 1 contract kept, 8 broken at error severity
and 1 broken at warn severity. No contract has an *indirect* layer or
independence violation; every layers and independence finding is a direct
import.

---

## `web-layers`

Container `web/src`. The layers, high to low, are `app` > `domains` >
`components` > `hooks` > `lib`. `web/src/mastra` and `web/src/config.ts` are
not layers.

**Violations**

| # | Importer | Imported | Specifier | Kind | Illegal pair |
|---|---|---|---|---|---|
| 1 | `web/src/lib/format.ts:1` | `web/src/domains/campaigns/types.ts` | `@/domains/campaigns/types` | value | lib → domains |
| 2 | `web/src/components/nav/Sidebar.tsx:3` | `web/src/app/(dashboard)/tabs.ts` | `@/app/(dashboard)/tabs` | value | components → app |

**Exempt**

- `web/src/lib/campaign-links.ts:2` → `web/src/domains/campaigns/api/client.ts`
  (lib → domains) is suppressed by `// zenith-ignore web-layers: …` on line 1.
  The suppression is used, so there is no unused-suppression warning.

**Not flagged, by design**

- `web/src/components/assistant.tsx:1` → `web/src/mastra/agent.ts`, because
  `mastra` is not a layer. `mastra-protected` covers it.
- Downward imports, for example
  `web/src/domains/campaigns/components/CampaignHeader.tsx:4` → `web/src/lib/format.ts`,
  `web/src/domains/analytics/components/AnalyticsChart.tsx:3`/`:4` →
  `web/src/lib/files.ts`/`web/src/lib/lazy.ts`, and
  `web/src/components/nav/Sidebar.tsx:4` → `web/src/hooks/use-debounce.ts`.
- No lower layer reaches a higher one indirectly, whether through `mastra`,
  `config.ts` or a workspace package.

## `package-tiers`

The layers are:

1. `pkg:web`, `pkg:sdk`, `apps/*` (`apps/portal`) and `services/*`
   (`services/mcp`), as independent siblings.
2. `pkg:@acme/ai` and `pkg:@acme/ui`, as independent siblings.
3. `pkg:@acme/api`.
4. `pkg:@acme/utils`.

`exclude_kinds = ["type"]`. `packages/legacyEditor` is in no layer.

**Violations**

| Importer | Imported | Specifier | Kind | Illegal pair |
|---|---|---|---|---|
| `packages/utils/src/campaign-url.ts:1` | `packages/api/index.ts` | `@acme/api` | value | utils → api |

**Exempt**

- `packages/utils/src/dates.ts:1` → `packages/api/index.ts` is `import type`,
  so `exclude_kinds` removes it.

**Not flagged, by design**

- `packages/ai/src/connectors/editor.ts:1` and
  `web/src/domains/editor/components/EditorCanvas.tsx:4` → `legacy-editor`,
  because legacy-editor is in no layer.
- No siblings import each other within layer 1 or layer 2.
- There are no indirect chains. When the search looks for utils → ai or
  utils → ui, the `api` layer is excluded, so the utils → api → … route is
  blocked.

## `packages-acyclic`

`pkg:*` must match **all 9** workspace packages, scoped ones included:
`packages/ai`, `packages/api`, `packages/legacyEditor`, `packages/ui`,
`packages/utils`, `services/mcp`, `apps/portal`, `sdk` and `web`.
`exclude_kinds = ["type"]`.

The edges between packages are counted as distinct file → file imports, after
type-only imports are excluded:

| From | To | Imports |
|---|---|---|
| `apps/portal` | `packages/api` | 1 |
| `apps/portal` | `packages/ui` | 2 |
| `apps/portal` | `packages/utils` | 1 |
| `packages/ai` | `packages/api` | 1 |
| `packages/ai` | `packages/legacyEditor` | 1 |
| `packages/api` | `packages/utils` | 2 |
| `packages/ui` | `packages/utils` | 1 |
| `packages/utils` | `packages/api` | 1 |
| `services/mcp` | `packages/ai` | 1 |
| `web` | `packages/ai` | 2 |
| `web` | `packages/api` | 5 |
| `web` | `packages/legacyEditor` | 1 |
| `web` | `packages/ui` | 2 |

`sdk` has no edges, because its only workspace import is type-only.

There is **one cycle**, `@acme/api` ⇄ `@acme/utils`:

- `@acme/api` → `@acme/utils`, 2 imports:
  `packages/api/src/domains/campaigns/api.ts:1` and
  `packages/api/src/domains/accounts/api.ts:1`, both to `packages/utils/index.ts`.
- `@acme/utils` → `@acme/api`, 1 import:
  `packages/utils/src/campaign-url.ts:1` → `packages/api/index.ts`.
  `packages/utils/src/dates.ts:1` is type-only and excluded.

**Expected cycle breaker:** `@acme/utils → @acme/api`, with 1 import. In
Eades–Lin–Smyth, api has δ = 2 − 1 = +1 and utils has δ = −1. So api is
placed first, and utils → api is the back edge. If the type-only import were
counted, both sides would be at 2, and the alphabetical tie-break, which also
puts api first, gives the same breaker.

The cycle also exists at file level:
`packages/utils/index.ts:1` → `packages/utils/src/campaign-url.ts:1` →
`packages/api/index.ts:1` → `packages/api/src/domains/campaigns/index.ts:2` →
`packages/api/src/domains/campaigns/api.ts:1` → `packages/utils/index.ts`.

## `client-safety`

**From:** the "use client" files under `web/src`. There are five:

- `web/src/domains/campaigns/components/CampaignHeader.tsx`
- `web/src/domains/campaigns/components/CampaignAiPanel.tsx`
- `web/src/domains/analytics/components/AnalyticsChart.tsx`
- `web/src/domains/editor/components/EditorCanvas.tsx`
- `web/src/components/nav/Sidebar.tsx`

`packages/ui/src/button.tsx` also says "use client", but it is outside
`web/src`. `.worktrees/copy/web/src/stale.ts` is gitignored.

**To:** `npm:sharp`, `npm:@mastra/core` (from `npm:@mastra/*`), `node:fs` and
`node:path` (from `node:*`), and `npm:server-only`.

`web/src/**` also matches every directory under `web/src`, and each
directory's unit contains these same files. Each chain must be reported
**once**, not once per overlapping from-unit.

**Chains.** Each is the unique shortest chain to its target. Once its edges are
excluded, no second chain exists.

**C1. To `npm:sharp`**, ending in a dynamic import:

1. `web/src/domains/campaigns/components/CampaignAiPanel.tsx:3` → `packages/ai/index.ts` (`@acme/ai`)
2. `packages/ai/index.ts:1` → `packages/ai/src/domains/campaigns/index.ts` (`export *`)
3. `packages/ai/src/domains/campaigns/index.ts:2` → `packages/ai/src/domains/campaigns/summarize.ts` (`export *`)
4. `packages/ai/src/domains/campaigns/summarize.ts:2` → `packages/ai/src/utils/images/to-model-image.ts`
5. `packages/ai/src/utils/images/to-model-image.ts:2` → `npm:sharp` (`await import("sharp")`, kind `dynamic`)

**C2. To `npm:@mastra/core`.** This chain exists only because the barrel
re-exports research. It is v1 barrel over-reporting, accepted in v1.

1. `web/src/domains/campaigns/components/CampaignAiPanel.tsx:3` → `packages/ai/index.ts`
2. `packages/ai/index.ts:3` → `packages/ai/src/domains/research/index.ts` (`export *`)
3. `packages/ai/src/domains/research/index.ts:1` → `npm:@mastra/core`

**C3. To `node:fs`:**

1. `web/src/domains/analytics/components/AnalyticsChart.tsx:3` → `web/src/lib/files.ts` (`@/lib/files`)
2. `web/src/lib/files.ts:1` → `node:fs`

**Targets that are never reached**

- `node:path`, which is imported only by `web/src/app/api/agents/route.ts:1`.
- `npm:server-only`, which is imported only by the two `api/server.ts` files.
  Only `page.tsx`, a server component, reaches them.

**Not flagged, by design**

- `web/src/domains/campaigns/components/CampaignHeader.tsx:5` →
  `web/src/domains/campaigns/api/actions.ts` is a
  "use server" file, which the contract's `boundaries` stop chains at. Its
  closure, which takes in `@acme/api` and the api ⇄ utils cycle, reaches
  nothing forbidden anyway; `tests/cases/features/forbidden-boundaries`
  covers a server action that does.
- `web/src/domains/campaigns/components/CampaignAiPanel.tsx:4` →
  `web/src/domains/campaigns/hooks/use-campaign-brief.ts:1` →
  `packages/ai/src/domains/campaigns/brief.ts:1` → `npm:zod`.
- `web/src/components/assistant.tsx` → `web/src/mastra/agent.ts` →
  `npm:@mastra/core`. `assistant.tsx` and its importer `Layout.tsx` are not
  "use client".

## `ai-public-surface` (severity `warn`)

The unit is `packages/ai`, and its only entry point is `packages/ai/index.ts`.

**Violations**

| Importer | Imported | Specifier |
|---|---|---|
| `web/src/domains/campaigns/hooks/use-campaign-brief.ts:1` | `packages/ai/src/domains/campaigns/brief.ts` | `@acme/ai/src/domains/campaigns/brief` |
| `services/mcp/src/tools.ts:1` | `packages/ai/src/tools/index.ts` | `@acme/ai/src/tools` |

`src/tools/index.ts` is named `index`, but it is not the unit-root `index`
entry point.

**Allowed:** `web/src/domains/campaigns/components/CampaignAiPanel.tsx:3` →
`packages/ai/index.ts`. Imports inside `packages/ai` are always allowed.

## `strategies-independent`

The units are `packages/ai/src/domains/content/strategies/{ads,blueprints,shared}`.
There is deliberately no `strategies/index.ts`, which would become a unit
itself.

**Violations**

| Importer | Imported | Specifier |
|---|---|---|
| `packages/ai/src/domains/content/strategies/ads/index.ts:1` | `packages/ai/src/domains/content/strategies/blueprints/index.ts` | `../blueprints` |

**Exempt through `ignore_imports`.** The entry matches 2 imports, so there is
no unmatched-ignore error.

- `packages/ai/src/domains/content/strategies/ads/index.ts:2` → `…/strategies/shared/index.ts`
- `packages/ai/src/domains/content/strategies/blueprints/index.ts:1` → `…/strategies/shared/index.ts`

**Not flagged:** `packages/ai/src/domains/content/index.ts:1`/`:2` → ads and
blueprints. The importer is outside every unit.

## `mastra-protected`

**Violations**

| Importer | Imported | Specifier |
|---|---|---|
| `web/src/components/assistant.tsx:1` | `web/src/mastra/agent.ts` | `@/mastra/agent` |

**Allowed**

- `web/src/app/api/agents/route.ts:2` → `web/src/mastra/agent.ts`, an allowed
  importer.
- `web/src/mastra/agent.ts:2` → `web/src/mastra/tools.ts`, an import inside
  the unit.
- Indirect access, such as `web/src/components/layout/Layout.tsx:3` →
  `web/src/components/assistant.tsx` → mastra, is not a violation, because `protected` checks direct imports only.

**Must not appear:** `.worktrees/copy/web/src/stale.ts:6` →
`web/src/mastra/agent.ts`. If it does, gitignore handling is broken.

## `api-domain-layers`

The containers are `packages/api/src/domains/accounts` and
`packages/api/src/domains/campaigns`. The layers in each are `hooks` (a
directory) > `api` (`api.ts`, matched extension-insensitively) > `types`
(`types.ts`). `index.ts` is in no layer. `exclude_kinds = ["type"]`.

**Violations**

| Importer | Imported | Specifier | Kind |
|---|---|---|---|
| `packages/api/src/domains/accounts/types.ts:1` | `packages/api/src/domains/accounts/api.ts` | `./api` | value |

**Exempt**

- `packages/api/src/domains/campaigns/types.ts:1` →
  `packages/api/src/domains/campaigns/api.ts` is `import type`, so
  `exclude_kinds` removes it.

**A trap that is avoided.** The domain `index.ts` barrels deliberately do not
re-export `hooks/`. If `campaigns/index.ts` did, the api ⇄ utils cycle would
create an indirect api → hooks chain:
`packages/api/src/domains/campaigns/api.ts:1` → `packages/utils/index.ts:1` →
`packages/utils/src/campaign-url.ts:1` → `packages/api/index.ts:1` →
`packages/api/src/domains/campaigns/index.ts` →
`packages/api/src/domains/campaigns/hooks/use-campaigns.ts`.

## `legacy-freeze`

**From:** `pkg:@acme/*`, which matches `packages/ai`, `packages/api`,
`packages/ui`, `packages/utils`, `services/mcp` and `apps/portal`.
**To:** `packages/legacyEditor`, the package named `legacy-editor`.

**Chain**

1. `packages/ai/src/connectors/editor.ts:1` → `packages/legacyEditor/index.ts` (`legacy-editor`)

Longer chains from other `packages/ai` files, such as
`packages/ai/src/domains/research/index.ts:2` →
`packages/ai/src/connectors/editor.ts`, reuse that last edge. After the first chain is found
that edge is excluded, so no other chain is found.

**Not flagged, by design**

- `web` is not in `from`. That covers
  `web/src/domains/editor/components/EditorCanvas.tsx:4` (value) and
  `web/src/domains/editor/types.ts:1` (type).
- `services/mcp` reaches only `packages/ai/src/tools/**`, which never reaches
  `connectors/editor.ts`.
- `apps/portal` never reaches `@acme/ai`.

## `sdk-standalone` (kept)

`sdk/src/client.ts:1` → `packages/api/index.ts` is `import type`, so
`exclude_kinds` removes it. No other path leads from `sdk` into `@acme/*`.
Without `exclude_kinds` the contract would break on that import.

---

## Resolution expectations

| Importer | Specifier | Expected target | What it exercises |
|---|---|---|---|
| `web/src/app/(dashboard)/layout.tsx:2` | `components/layout/Layout` | `web/src/components/layout/Layout.tsx` | web's catch-all `*` path |
| `web/src/app/layout.tsx:2` | `config` | `web/src/config.ts` | the catch-all shadows the npm package `config`, which web declares. It is **not** `npm:config` |
| `web/src/lib/format.ts:1` | `@/domains/campaigns/types` | `web/src/domains/campaigns/types.ts` | web's `@/*` |
| `apps/portal/src/App.tsx:3` | `@/routes/home` | `apps/portal/src/routes/home.tsx` | portal's own `@/*`, from per-file tsconfig discovery |
| `apps/portal/src/App.tsx:2` | `@/routes/campaigns.$id` | `apps/portal/src/routes/campaigns.$id.tsx` | a TanStack-style `$` file name |
| `web/src/domains/campaigns/api/client.ts:1` | `@acme/api` | `packages/api/index.ts` | workspace `main` |
| `web/src/domains/campaigns/api/server.ts:2` | `@acme/api/src/domains/campaigns/api` | `packages/api/src/domains/campaigns/api.ts` | a deep workspace import |
| `web/src/domains/campaigns/hooks/use-campaign.ts:1` | `@acme/api/src/domains/campaigns/hooks/use-campaigns` | `packages/api/src/domains/campaigns/hooks/use-campaigns.ts` | a deep workspace import |
| `services/mcp/src/tools.ts:1` | `@acme/ai/src/tools` | `packages/ai/src/tools/index.ts` | a deep import of a directory index |
| `web/src/domains/campaigns/components/CampaignHeader.tsx:3` | `@acme/ui` | `packages/ui/index.ts` | `exports["."]` |
| `web/src/domains/editor/components/EditorCanvas.tsx:3` | `@acme/ui/button` | `packages/ui/src/button.tsx` | `exports["./button"]`. No `packages/ui/button.*` exists, so only the exports map resolves it |
| `apps/portal/src/App.tsx:1` | `@acme/ui/button` | `packages/ui/src/button.tsx` | the same |
| `packages/ai/src/connectors/editor.ts:1` | `legacy-editor` | `packages/legacyEditor/index.ts` | a package name that differs from its directory |
| `services/mcp/src/index.ts:1` | `./server.js` | `services/mcp/src/server.ts` | `.js` → `.ts` |
| `services/mcp/src/server.ts:2` | `./tools.js` | `services/mcp/src/tools.ts` | `.js` → `.ts` |
| `services/mcp/src/server.ts:4` | `typeof import("./tools.js")` | `services/mcp/src/tools.ts` | kind `type` |
| `services/mcp/src/server.ts:1` | `node:fs/promises` | `node:fs` | a builtin subpath, squashed |
| `web/src/app/api/agents/route.ts:1` | `path` | `node:path` | a bare builtin, normalised |
| `web/src/mastra/agent.ts:1` | `@mastra/core/agent` | `npm:@mastra/core` | an npm subpath, squashed |
| `apps/portal/src/main.tsx:1` | `react-dom/client` | `npm:react-dom` | an npm subpath, squashed |
| `web/tailwind.config.js:1` | `require("tailwindcss/plugin")` | `npm:tailwindcss` | kind `require`, in CommonJS |
| `packages/ai/src/tools/research.ts:2` | `../liquid/prompt.liquid` | `packages/ai/src/liquid/prompt.liquid` | a non-source file node |
| `packages/utils/src/currency.ts:1` | `./data.json` | `packages/utils/src/data.json` | a JSON import |
| `web/src/components/ui/Logo.tsx:1` | `./logo.svg` | `web/src/components/ui/logo.svg` | an SVG import |
| `web/src/app/layout.tsx:3` | `./styles.css` | `web/src/app/styles.css` | a CSS side-effect import |
| `web/src/lib/lazy.ts:2` | `` import(`./lazy-chart`) `` | `web/src/lib/lazy-chart.ts` | kind `dynamic`, from a substitution-free template |

**Kinds worth asserting**

- **`type`:**
  - `web/src/domains/campaigns/types.ts:1` (`import { type Campaign }`, where every specifier is type-only)
  - `packages/api/src/domains/campaigns/types.ts:1` (`import type`)
  - `packages/api/index.ts:3` (`export type { … } from`)
  - `services/mcp/src/server.ts:4` (`typeof import`)
  - `packages/utils/src/dates.ts:1`
  - `sdk/src/client.ts:1`
- **`value`, despite a `type` modifier:** `packages/api/src/domains/campaigns/api.ts:2`
  (`import { type Campaign, CAMPAIGN_STATUSES }`).
- **`value` namespace re-exports:** `packages/api/index.ts:1`/`:2` (`export * as campaigns`, `export * as accounts`).
- **`dynamic`:** `packages/ai/src/utils/images/to-model-image.ts:2` and `web/src/lib/lazy.ts:2`.
- **`require`:** `web/tailwind.config.js:1`.
- **Side-effect `value`:** `web/src/domains/campaigns/api/server.ts:1` and
  `web/src/domains/analytics/api/server.ts:1` (`import "server-only"`), plus the CSS imports.

**Directives**

- "use client":
  - `web/src/domains/campaigns/components/CampaignHeader.tsx`
  - `web/src/domains/campaigns/components/CampaignAiPanel.tsx`
  - `web/src/domains/analytics/components/AnalyticsChart.tsx`
  - `web/src/domains/editor/components/EditorCanvas.tsx`
  - `web/src/components/nav/Sidebar.tsx`
  - `packages/ui/src/button.tsx`
- "use server": `web/src/domains/campaigns/api/actions.ts`.

## Unresolved imports

- **Relative imports that fail.** These are warnings and are not in the graph:
  - `sdk/src/client.ts:3`: `./does-not-exist`
- **Non-literal imports.** These are skipped, counted, and listed with `--verbose`:
  - `web/src/lib/lazy.ts:6`: `import(name)`
- **Bare specifiers with no `node_modules`.** These become `npm:` nodes flagged
  unresolved: 10 packages and 21 imports.

| Node | Importers |
|---|---|
| `npm:@mastra/core` | `packages/ai/src/domains/research/index.ts:1`, `web/src/mastra/agent.ts:1`, `web/src/mastra/tools.ts:1` |
| `npm:@tanstack/react-query` | `packages/api/src/domains/accounts/hooks/use-accounts.ts:1`, `packages/api/src/domains/campaigns/hooks/use-campaigns.ts:1` |
| `npm:@vitejs/plugin-react` | `apps/portal/vite.config.ts:1` |
| `npm:react` | `web/src/app/layout.tsx:1`, `web/src/app/(dashboard)/layout.tsx:1`, `web/src/components/layout/Layout.tsx:1`, `web/src/domains/analytics/hooks/use-analytics.ts:1`, `web/src/hooks/use-debounce.ts:1` |
| `npm:react-dom` | `apps/portal/src/main.tsx:1` |
| `npm:server-only` | `web/src/domains/analytics/api/server.ts:1`, `web/src/domains/campaigns/api/server.ts:1` |
| `npm:sharp` | `packages/ai/src/utils/images/to-model-image.ts:2` |
| `npm:tailwindcss` | `web/tailwind.config.js:1` |
| `npm:vite` | `apps/portal/vite.config.ts:2` |
| `npm:zod` | `packages/ai/src/domains/campaigns/brief.ts:1`, `packages/ai/src/tools/research.ts:1`, `sdk/src/client.ts:2`, `web/src/mastra/tools.ts:2` |

No workspace import is unresolved, `@acme/ui/button` included, and neither is
`config`.

## Columns of expression imports

| Location | Statement column | Expression column |
|---|---|---|
| `web/tailwind.config.js:1` | 1 | 16 (`require(`) |
| `web/src/lib/lazy.ts:2` | 3 | 10 (`import(`) |
| `web/src/lib/lazy.ts:6` (non-literal) | 3 | 10 (`import(`) |
| `packages/ai/src/utils/images/to-model-image.ts:2` | 3 | 36 (`import(`). `await` is at 30 |
| `services/mcp/src/server.ts:4` | 1 | 20 (`typeof`), 27 (`import(`) |

## Gitignored files

`.worktrees/copy/web/src/stale.ts` has "use client", imports `sharp` on line
5, and imports `web/src/mastra/agent` relatively on line 6. It must not be
scanned. If it is, three things change:

- `mastra-protected` gains `.worktrees/copy/web/src/stale.ts:6` →
  `web/src/mastra/agent.ts`.
- `npm:sharp` gains an importer.
- The scan finds 79 source files instead of 78.

`client-safety` does not change, because the file is outside `web/src`.
