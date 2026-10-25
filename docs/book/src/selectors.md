# Selectors

A selector picks the units that a contract is about. It is a string, or a
table for the advanced forms.

## String forms

| Form | Matches | Example |
|---|---|---|
| path glob | directories and files in the project | `web/src/domains/*`, `packages/ai/src/**/server.ts` |
| `pkg:<name>` | the directory of each matching package, of any language | `pkg:@acme/*`, `pkg:example.com/api` |
| `<scheme>:<name>` | the external packages or builtins of a language | `npm:sharp`, `node:*`, `gomod:github.com/pkg/*`, `crate:serde` |

Each scheme belongs to a [language](languages.md), whose page lists its
schemes, so using one while its language is not enabled is an error, as is any
other scheme that zenith doesn't know.

`pkg:*` never matches the root package, which would contain everything.

Package, external and builtin names are matched with name patterns, in which
`*` matches any run of characters, `/` included, so `npm:*` matches scoped
packages like `@tanstack/react-query`.

When a path glob matches nested nodes, such as `web/**` matching both
`web/src` and `web/src/a.ts`, only the outermost become units, since they
already contain the others.

## Glob syntax

zenith's glob dialect needs no escaping for Next.js and TanStack file names:

- `/` separates segments. A leading `./` is ignored.
- `*` matches any run of characters within one segment.
- `**` as a whole segment matches zero or more segments; a trailing `/**`
  matches one or more, so `a/**` means strictly inside `a`.
- `?` matches one character, `{a,b}` either alternative, and `\` escapes the
  next character.
- `[`, `]`, `(`, `)`, `$`, `@` and `+` are literal, so
  `web/src/app/(dashboard)/[id]/page.tsx` matches that file.
- Files match with or without the extension of an enabled language, such as
  `.ts` for `js`, so `src/types` matches both the directory `src/types` and the
  file `src/types.ts`.

## The table form

```toml
{ path = "web/src/**", directive = "use client" }
{ path = "web/src/features", except = ["web/src/features/experimental"] }
```

- `path` is any string-form selector.
- `except` removes the members that its selectors match from each unit. A
  unit that it empties takes part in nothing, so `except` also removes whole
  units: in `{ path = "src/parse/*", except = ["src/parse/mod.rs"] }`, the
  directory's own `mod.rs` is not a sibling of the modules in it.
- `directive` keeps only the files whose directives include it, exactly as
  written, and makes **each file a unit of its own**, so a violation names the
  file. It only applies to path and `pkg:` selectors.

## Selectors that match nothing

A path or `pkg:` selector that matches nothing is a configuration error, since
it is most likely a typo or a rename. The selectors of a language's scheme, such
as `npm:sharp`, may match nothing: it only means that nothing imports the
package. The check is made
before `except` and `directive` filter the units, and never for
`ignore_imports` entries.
