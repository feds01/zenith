# Getting started

## 1. Write a configuration

In the root of your repository, run:

```sh
zenith init
```

It writes a `zenith.toml` for the workspace packages it finds. With two or
more packages, its one contract keeps them free of dependency cycles:

```toml
#:schema https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json

[contracts.packages-acyclic]
type = "acyclic"
modules = ["pkg:*"]
```

The `#:schema` line lets editors with a TOML language server (Taplo, or Even
Better TOML in VS Code) complete and check the file.

zenith reads JavaScript and TypeScript by default. For a project in any other
language, or one that mixes several, enable each language with its table, such
as `[languages.python]`: see [languages](languages.md).

## 2. Check the contracts

```sh
zenith check
```

zenith finds the project's source files, parses and resolves their imports
in parallel, and checks every contract against the import graph. Each
violation is shown with the code that makes it, then a line per contract and a
summary:

```text
Contracts
  ✗ packages-acyclic  acyclic       broken (1 violation)

0 kept, 1 broken · 2 files, 2 imports
```

`zenith check` exits with 1 when a contract is broken, so it can fail a CI job,
as with zenith's [GitHub Action](ci.md).

## 3. Add contracts

Describe the architecture you want, a rule at a time: see the
[contract types](contracts/overview.md). Two commands help while writing them:

- `zenith graph web/src --depth 1` shows a directory's children and the
  imports between them.
- `zenith why web/src/lib web/src/app` shows why one part imports another,
  chain by chain.

## 4. Adopt it on an existing codebase

A large codebase usually breaks its contracts in many places at first.
`zenith baseline` records the existing violations, so that `zenith check` only
fails on new ones, and the [baseline](baseline.md) shrinks as they are fixed.
