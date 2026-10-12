# protected

Only the `allowed_importers`, and the protected units' own members, may
import a `modules` unit **directly**.

| Option | Default | |
|---|---|---|
| `modules` | required | Selectors of the protected units. |
| `allowed_importers` | required | Selectors. |

```toml
[contracts.agents-are-api-only]
type = "protected"
modules = ["web/src/agents"]
allowed_importers = ["web/src/app/api"]
```

Say `web/src/agents` holds LLM agents that read API keys and call paid
services, and should only run behind API routes:

- **Broken by** `web/src/components/chat.tsx` importing `@/agents/support`.
- **Allowed:** `web/src/app/api/chat/route.ts` importing an agent, and the
  agents importing each other.
- **Not checked:** a component importing `web/src/lib/run.ts`, which imports
  an agent. `protected` only checks direct imports, to keep the list of
  importers short and deliberate; a [`forbidden`](forbidden.md) contract can
  rule out the chains as well.
