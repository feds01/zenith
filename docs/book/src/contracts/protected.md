# protected

Only the `allowed_importers`, and the protected units' own members, may
import a `modules` unit **directly**.

## Example

```toml
[contracts.agents-are-api-only]
type = "protected"
modules = ["web/src/agents"]
allowed_importers = ["web/src/app/api"]
```

<figure class="diagram">
{{#include ../diagrams/protected.svg}}
<figcaption>Only the API route, and the agents themselves, may import an agent directly.</figcaption>
</figure>

Say `web/src/agents` holds LLM agents that read API keys and call paid
services, and should only run behind API routes:

- **Broken by** `web/src/components/chat.tsx` importing `@/agents/support`,
  and by `web/src/lib/run.ts` importing `@/agents/tools`.
- **Allowed:** `web/src/app/api/chat/route.ts` importing an agent, and the
  agents importing each other.
- **Not checked:** `chat.tsx` importing `run.ts`, which isn't protected, so
  the chain from `chat.tsx` to an agent isn't reported, only `run.ts`'s own
  import. `protected` only checks direct imports, to keep the list of
  importers short and deliberate; a [`forbidden`](forbidden.md) contract can
  rule out the chains as well.

## Restricting a third-party dependency

A [language's scheme](../selectors.md#string-forms) protects an external
package, so that only some of your code may use it:

```toml
[contracts.sharp-only-in-images]
type = "protected"
modules = ["npm:sharp"]
allowed_importers = ["web/src/images"]
```

Say `sharp`, a native image library, should stay behind one module that
resizes images:

- **Broken by** `web/src/app/profile/avatar.ts` importing `sharp`.
- **Allowed:** `web/src/images/resize.ts` importing `sharp`, and `avatar.ts`
  importing `web/src/images/resize.ts`.

Other languages' packages work the same, such as
`gomod:github.com/disintegration/imaging` or `crate:serde`. Since such a
selector may match nothing, a misspelt package name isn't an error: the
contract is simply kept.

## Options

| Option | Default | |
|---|---|---|
| `modules` | required | Selectors of the protected units. |
| `allowed_importers` | required | Selectors. |

## Common options

Like every contract type, `protected` also takes:

{{#include overview.md:common-options}}
