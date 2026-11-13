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
packages like `@tanstack/react-query`. A scheme whose names are paths, as
`rust:` is for Rust's standard library, also matches everything under the
names it matches, so `rust:std` and `rust:std::thread` both match
`rust:std::thread::spawn`.

When a path glob matches nested nodes, such as `web/**` matching both
`web/src` and `web/src/a.ts`, only the outermost become units, since they
already contain the others. In Rust, a file and the directory beside it of
the same name, such as `src/crates.rs` and `src/crates/`, are one module, so
they are one unit, labelled `src/crates`, whichever of them a glob matches.

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
{ path = "web/src/app", except = [{ path = "web/src/app", directive = "use client" }] }
{ path = "npm:react", names = ["useState", "useEffect"] }
```

- `path` is any string-form selector.
- `except` removes the members that its selectors match from each unit. A
  unit that it empties takes part in nothing, so `except` also removes whole
  units: in `{ path = "src/parse/*", except = ["src/parse/mod.rs"] }`, the
  directory's own `mod.rs` is not a sibling of the modules in it. Its
  selectors take either form, so a table with a `directive` removes the files
  that have it: the last example above is `web/src/app` as one unit, without
  its client components.
- `directive` keeps only the files whose directives include it, exactly as
  written, and makes **each file a unit of its own**, so a violation names the
  file. It only applies to path and `pkg:` selectors.
- `names` keeps the units, but only the import statements that take one of
  the names, as a value, count as imports of them, and a report points at
  those statements. The default import is `default`. A statement that takes
  the whole module doesn't count: a namespace import, a side-effect import,
  `export *`, a dynamic import or a `require` call. Nor does a name that
  only its type is imported for, as in `import { type A }` or
  `import type { A }`.

`names` only applies where a selector picks what is imported: `to` in
[`forbidden`](contracts/forbidden.md), and `modules` in
[`protected`](contracts/protected.md). Imports record the names they take in
JavaScript, TypeScript and Rust, so `names` is an error on the schemes of
the other languages, such as `pypi:` or `gomod:`. On a path or `pkg:`
selector, it only matches the imports of files whose language records
names. A unit that several selectors match counts the imports that any of
them counts, so one without `names` counts every import.

A module reached through a pass-through module, such as a barrel of
`export *` lines, is imported by the barrel, which takes the whole module,
so `names` on it never matches. With
[`reexports = "transparent"`](concepts.md#re-exports), the import of it is the
one that takes the names: use it for a `names` selector of a module that a
barrel re-exports.

## Selectors that match nothing

A path or `pkg:` selector that matches nothing is a configuration error, since
it is most likely a typo or a rename. The selectors of a language's scheme, such
as `npm:sharp`, may match nothing: it only means that nothing imports the
package. The check is made
before `except` and `directive` filter the units, and never for
`ignore_imports` entries.
