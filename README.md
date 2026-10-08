# Zenith

> An architecture linter for TypeScript and JavaScript codebases, built for monorepos.

Zenith enforces *contracts* about which parts of a codebase may import which
other parts, such as layers, forbidden dependencies, independent modules,
protected modules, acyclic packages and public entry points. You declare them
in `zenith.toml`, and `zenith check` fails when the code breaks them, pointing
at the exact import, or the full import chain when the dependency is indirect.

The contract model comes from Python's
[import-linter](https://github.com/seddonym/import-linter), and the graph
algorithms are ports of its engine [grimp](https://github.com/python-grimp/grimp).
Both are adapted to how JS/TS projects are organised: files and directories,
workspace packages, `tsconfig` path aliases, `exports` maps, type-only imports
and `"use client"` boundaries. Zenith is a single Rust binary built on
[oxc](https://oxc.rs). On a 6,000-file monorepo it builds the full import graph
and checks 15 contracts in about 0.2 seconds.

```
error[client-safety]: `web/src/domains/campaigns/components/CampaignAiPanel.tsx` must not import `npm:sharp`
 --> web/src/domains/campaigns/components/CampaignAiPanel.tsx:3:35
  |
3 | import { summarizeCampaign } from "@acme/ai";
  |                                   ^^^^^^^^^^ starts a chain of 5 imports
  |
  = note: import chain:
            web/src/domains/campaigns/components/CampaignAiPanel.tsx:3
            -> packages/ai/index.ts:1
            -> packages/ai/src/domains/campaigns/index.ts:2
            -> packages/ai/src/domains/campaigns/summarize.ts:2
            -> packages/ai/src/utils/images/to-model-image.ts:2 (dynamic)
            -> npm:sharp
```

## Installation

Prebuilt binaries are attached to every [GitHub release](https://github.com/feds01/zenith/releases).

macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/feds01/zenith/releases/latest/download/zenith-installer.sh | sh
```

Windows:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/feds01/zenith/releases/latest/download/zenith-installer.ps1 | iex"
```

From source:

```sh
cargo install --git https://github.com/feds01/zenith zenith
```

## Usage

```sh
zenith init          # write a starter zenith.toml from the workspace packages
zenith check         # check every contract; exit 1 if any is broken
zenith baseline      # record today's violations, so only new ones fail
```

A `zenith.toml` with a few contracts:

```toml
#:schema https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json

# Workspace packages form tiers: apps on top, shared utilities at the bottom.
[contracts.package-tiers]
type = "layers"
layers = [
  "pkg:web | apps/* | services/*",
  "pkg:@acme/ai | pkg:@acme/ui",
  "pkg:@acme/api",
  "pkg:@acme/utils",
]
exclude_kinds = ["type"]          # type-only imports are erased at runtime

# No dependency cycles between workspace packages.
[contracts.packages-acyclic]
type = "acyclic"
modules = ["pkg:*"]

# Client components never reach server-only code, even through barrels.
[contracts.client-safety]
type = "forbidden"
from = [{ path = "web/src/**", directive = "use client" }]
to = ["npm:sharp", "node:*", "npm:server-only"]
guidance = "Move the call into a server action."

# Other packages may only import @acme/ai through its entry point.
[contracts.ai-public-surface]
type = "encapsulation"
modules = ["pkg:@acme/ai"]
entrypoints = ["index"]
```

| Contract | Rule |
|---|---|
| `layers` | Higher layers may import lower ones, never the reverse. Supports per-container layers, independent (`a \| b`) and non-independent (`a : b`) siblings, optional `(a)` and closed layers, and exhaustiveness checks. |
| `forbidden` | The `from` modules must not import the `to` modules, directly or transitively. |
| `independence` | The modules must not depend on one another. |
| `protected` | Only the allowed importers may directly import the modules. |
| `acyclic` | No dependency cycles between the modules (and, with `depth`, their children). Nominates the cheapest dependencies to cut. |
| `encapsulation` | Outside code may only import a module through its entry points. |

Selectors are path globs (`web/src/domains/*`), workspace packages
(`pkg:@acme/*`), external packages (`npm:react`) or builtins (`node:*`). Brackets
and parentheses are literal, so Next.js paths like `app/(dashboard)/[id]` need
no escaping.

To adopt zenith on a codebase that already breaks its contracts:

- **`zenith baseline`** records the current violations in
  `zenith.baseline.json`. Only new violations then fail the check, and entries
  that get fixed must be removed (`zenith baseline --prune`), so the debt can
  only shrink.
- **Inline suppressions** silence a single import:
  `// zenith-ignore <contract-id>: reason`.
- **`severity = "warn"`** reports a contract without failing the check.

Debugging helpers:

```sh
zenith why pkg:web npm:sharp                         # how does web reach sharp?
zenith graph web/src --depth 1                       # the module tree, and dependencies between children
zenith resolve web/src/lib/a.ts @/domains/campaigns  # how one specifier resolves
zenith check --format json | github | compact        # machine-readable output
```

The full design, with the exact semantics of every contract, selector and
option, is in [docs/SPEC.md](docs/SPEC.md).

### Development

Zenith builds with a pinned nightly toolchain (see `rust-toolchain.toml`) and
uses [just](https://github.com/casey/just):

```sh
just setup              # install the toolchain and the pre-commit hooks
just test               # all tests
just test-ui acme       # UI snapshot tests in tests/cases, filtered by name
just update-snapshots   # re-record the UI snapshots
just lint               # clippy (deny warnings) and the formatting check
just run check          # run the CLI from source in the current directory
```

See [CLAUDE.md](CLAUDE.md) for the architecture and conventions.

### Releasing

Bump `version` in `crates/zenith/Cargo.toml`, merge to `main`, then run
`just release` to tag it. [cargo-dist](https://opensource.axo.dev/cargo-dist/)
builds the binaries and installers and publishes the GitHub release.

## License

MIT
