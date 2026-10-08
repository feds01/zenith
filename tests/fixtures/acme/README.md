# acme fixture

`acme` is a small synthetic bun monorepo with about 80 source files. It
reproduces the patterns of zenith's large reference monorepo (spec §12):

- workspace packages whose names differ from their directories
- `@/*` and catch-all `*` tsconfig paths
- an `exports` map
- deep `@acme/x/src/...` imports and namespace barrels
- `"use client"` and `"use server"` files
- type-only imports
- Next.js `(group)` and `[param]` directories
- `.js` → `.ts` imports and asset imports
- a gitignored `.worktrees/` copy

It contains deliberate architecture violations. [`zenith.toml`](zenith.toml)
has one contract for each, and [`VIOLATIONS.md`](VIOLATIONS.md) lists every
finding, chain, exemption and unresolved import, with `file:line`. Source files
are deliberately tiny. If you change one, re-check its line numbers in
`VIOLATIONS.md`.

> The zenith repo's root `.gitignore` also ignores `.worktrees/`.
> `.worktrees/copy/web/src/stale.ts` must therefore be force-added
> (`git add -f`), or re-included with `!/tests/fixtures/acme/.worktrees/`.

## Import-style coverage

| Style | Where |
|---|---|
| relative | `web/src/components/layout/Layout.tsx:3` (`../assistant`) |
| `@/` alias (web) | `web/src/lib/format.ts:1` |
| `@/` alias (portal, its own tsconfig) | `apps/portal/src/App.tsx:3` |
| bare specifier through the catch-all `*` | `web/src/app/(dashboard)/layout.tsx:2` (`components/layout/Layout`) |
| catch-all `*` shadowing an npm package | `web/src/app/layout.tsx:2` (`config` resolves to `web/src/config.ts`) |
| workspace root barrel | `web/src/domains/campaigns/api/client.ts:1` (`import { campaigns } from "@acme/api"`) |
| deep workspace import | `web/src/domains/campaigns/api/server.ts:2` (`@acme/api/src/domains/campaigns/api`) |
| `exports` map, root | `web/src/domains/campaigns/components/CampaignHeader.tsx:3` (`@acme/ui`) |
| `exports` map, subpath | `web/src/domains/editor/components/EditorCanvas.tsx:3` (`@acme/ui/button`) |
| package name differing from its directory | `packages/ai/src/connectors/editor.ts:1` (`legacy-editor` resolves to `packages/legacyEditor`) |
| `import type { … }` | `packages/api/src/domains/campaigns/types.ts:1` |
| inline type specifiers only (kind `type`) | `web/src/domains/campaigns/types.ts:1` (`import { type Campaign }`) |
| mixed inline (kind `value`) | `packages/api/src/domains/campaigns/api.ts:2` (`import { type Campaign, CAMPAIGN_STATUSES }`) |
| `export type { … } from` | `packages/api/index.ts:3` |
| `export * from` | `packages/ai/index.ts:1` |
| `export * as ns from` | `packages/api/index.ts:1` |
| dynamic `await import("sharp")` | `packages/ai/src/utils/images/to-model-image.ts:2` |
| dynamic, substitution-free template | `web/src/lib/lazy.ts:2` (`` import(`./lazy-chart`) ``) |
| dynamic, non-literal (skipped) | `web/src/lib/lazy.ts:6` (`import(name)`) |
| `require()` in CommonJS | `web/tailwind.config.js:1` |
| `typeof import()` type | `services/mcp/src/server.ts:4` |
| side-effect `import "server-only"` | `web/src/domains/campaigns/api/server.ts:1` |
| CSS | `web/src/app/layout.tsx:3` (`./styles.css`) |
| SVG | `web/src/components/ui/Logo.tsx:1` |
| JSON | `packages/utils/src/currency.ts:1` (`./data.json`) |
| `.liquid` template | `packages/ai/src/tools/research.ts:2` (`../liquid/prompt.liquid`) |
| `.js` → `.ts` (`"type": "module"` package) | `services/mcp/src/server.ts:2` (`./tools.js`) |
| builtin `node:fs` | `web/src/lib/files.ts:1` |
| builtin subpath `node:fs/promises` | `services/mcp/src/server.ts:1` |
| bare builtin `path` | `web/src/app/api/agents/route.ts:1` |
| npm packages and subpaths | `react` (`web/src/hooks/use-debounce.ts:1`), `zod` (`packages/ai/src/domains/campaigns/brief.ts:1`), `@mastra/core/agent` (`web/src/mastra/agent.ts:1`), `react-dom/client` (`apps/portal/src/main.tsx:1`) |
| missing relative file | `sdk/src/client.ts:3` (`./does-not-exist`) |
| `"use client"` | `web/src/domains/campaigns/components/CampaignAiPanel.tsx:1`, and four more web files |
| `"use server"` | `web/src/domains/campaigns/api/actions.ts:1` |
| inline suppression | `web/src/lib/campaign-links.ts:1` |
| Next.js `(group)` and `[param]` | `web/src/app/(dashboard)/campaigns/[id]/page.tsx` |
| TanStack-style `$param` file | `apps/portal/src/routes/campaigns.$id.tsx` |
| gitignored file | `.worktrees/copy/web/src/stale.ts` |
