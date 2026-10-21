# Selectors

A selector picks the units that a contract is about. It is a string, or a
table for the advanced forms.

## String forms

| Form | Matches | Example |
|---|---|---|
| path glob | directories and files in the project | `web/src/domains/*`, `packages/ai/src/**/server.ts` |
| `pkg:<name>` | the directory of each matching workspace package | `pkg:@acme/*`, `pkg:web` |
| `npm:<name>` | external packages | `npm:sharp`, `npm:@aws-sdk/*` |
| `node:<name>`, `bun:<name>` | builtins | `node:*`, `node:fs` |
| `gomod:<module>` | Go modules that are not in the project | `gomod:github.com/pkg/*` |
| `go:<package>` | Go standard library packages | `go:net/http`, `go:os/*` |

`npm:`, `node:` and `bun:` belong to the `js` language, and `go:` and `gomod:`
to `go`, so using one while its language is not
[enabled](configuration.md#languages) is an error, as is any other scheme that
zenith doesn't know.

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
- `except` removes the members that its selectors match from each unit.
- `directive` keeps only the files whose directives include it, exactly as
  written, and makes **each file a unit of its own**, so a violation names the
  file. It only applies to path and `pkg:` selectors.

## Selectors that match nothing

A path or `pkg:` selector that matches nothing is a configuration error, since
it is most likely a typo or a rename. `npm:`, `node:`, `go:` and `gomod:`
selectors may match nothing: it only means that nothing imports the package. The check is made
before `except` and `directive` filter the units, and never for
`ignore_imports` entries.
