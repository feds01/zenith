# Changelog

What changed for zenith's users in each release. dist shows a release's
section as its notes on GitHub.

## [0.6.0] - 2026-10-25

[Changes since v0.5.0](https://github.com/feds01/zenith/compare/v0.5.0...v0.6.0)

### Features

- **check:** add a structured fix hint to each violation ([#124](https://github.com/feds01/zenith/pull/124))
- **graph:** add a mermaid output format ([#123](https://github.com/feds01/zenith/pull/123))
- **check:** add a SARIF output format ([#126](https://github.com/feds01/zenith/pull/126))
- **check:** add --since <ref> to report only what a branch changed ([#127](https://github.com/feds01/zenith/pull/127))
- **contracts:** add a limits contract for fan-in and fan-out ([#125](https://github.com/feds01/zenith/pull/125))
- **ci:** add a GitHub Action that checks, annotates and uploads SARIF ([#130](https://github.com/feds01/zenith/pull/130))
- **suggest:** propose contracts from the import graph ([#129](https://github.com/feds01/zenith/pull/129))
- **graph:** add 'graph diff' to show how dependencies changed between refs ([#132](https://github.com/feds01/zenith/pull/132))
- **pipeline:** read a project's files through a file system interface ([#135](https://github.com/feds01/zenith/pull/135))
- **lang:** read every language's files through the session's file system ([#136](https://github.com/feds01/zenith/pull/136))
- **graph:** read 'graph diff' refs from git rather than checking them out ([#137](https://github.com/feds01/zenith/pull/137))
- **lsp:** add 'zenith lsp' and a VS Code extension to show violations in the editor ([#131](https://github.com/feds01/zenith/pull/131))
- **suggest:** group same-level layers, skip covered contracts, and suggest from init ([#147](https://github.com/feds01/zenith/pull/147))
- **config:** leave each language's index files out of exhaustive layers ([#150](https://github.com/feds01/zenith/pull/150))
- **rust:** name the paths into the standard library ([#152](https://github.com/feds01/zenith/pull/152))
- **lsp:** re-check incrementally, keeping the graph between saves ([#156](https://github.com/feds01/zenith/pull/156))
- **rust:** a test import kind for what only tests compile ([#155](https://github.com/feds01/zenith/pull/155))
- **rust:** see imports through re-exports with reexports = "transparent" ([#159](https://github.com/feds01/zenith/pull/159))
- **rust:** make foo.rs and foo/ one unit ([#163](https://github.com/feds01/zenith/pull/163))

### Fixes

- **workspace:** stop the whole git process in the test of a stopped reader ([#140](https://github.com/feds01/zenith/pull/140))
- **contracts:** read the tests' configurations with Rust too ([#162](https://github.com/feds01/zenith/pull/162))
- **rust:** read a::m as the item m of a when a re-exports it ([#160](https://github.com/feds01/zenith/pull/160))

### Documentation

- **protected:** show how to restrict a third-party dependency ([#122](https://github.com/feds01/zenith/pull/122))
- **book:** document boundaries, and that except can remove a unit ([#151](https://github.com/feds01/zenith/pull/151))

## [0.5.0] - 2026-10-23

[Changes since v0.4.0](https://github.com/feds01/zenith/compare/v0.4.0...v0.5.0)

### Features

- **terraform:** read the module sources of Terraform and Terragrunt files ([#105](https://github.com/feds01/zenith/pull/105))
- **terraform:** resolve module sources to directories and external modules ([#106](https://github.com/feds01/zenith/pull/106))
- **terraform:** check Terraform and Terragrunt projects ([#107](https://github.com/feds01/zenith/pull/107))

## [0.4.0] - 2026-10-23

[Changes since v0.3.0](https://github.com/feds01/zenith/compare/v0.3.0...v0.4.0)

### Features

- **diagnostics:** keep what zenith says in a Fluent catalogue ([#88](https://github.com/feds01/zenith/pull/88))
- **python:** check Python projects ([#89](https://github.com/feds01/zenith/pull/89))
- **rust:** stop reading quote! bodies and #interpolations as imports ([#95](https://github.com/feds01/zenith/pull/95))
- **diagnostics:** ranges, numbers and compile-fail tests for the catalogue ([#96](https://github.com/feds01/zenith/pull/96))

### Fixes

- **resolve:** link workspace packages into the root node_modules ([#86](https://github.com/feds01/zenith/pull/86))
- **rust:** fewer false syntax errors and unresolved imports ([#87](https://github.com/feds01/zenith/pull/87))

## [0.3.0] - 2026-10-22

[Changes since v0.2.0](https://github.com/feds01/zenith/compare/v0.2.0...v0.3.0)

### Features

- **go:** read the imports of Go files ([#71](https://github.com/feds01/zenith/pull/71))
- **go:** resolve Go import paths against the project's modules ([#72](https://github.com/feds01/zenith/pull/72))
- **graph:** let imports target a package directory ([#76](https://github.com/feds01/zenith/pull/76))
- **Breaking:** **config:** configure each language in its own table ([#77](https://github.com/feds01/zenith/pull/77))
- **pipeline:** run each language through a backend ([#79](https://github.com/feds01/zenith/pull/79))
- **go:** check Go projects ([#80](https://github.com/feds01/zenith/pull/80))
- **rust:** check Rust projects ([#82](https://github.com/feds01/zenith/pull/82))

### Documentation

- describe zenith as a linter for several languages ([#81](https://github.com/feds01/zenith/pull/81))

## [0.2.0] - 2026-10-14

[Changes since v0.1.0](https://github.com/feds01/zenith/compare/v0.1.0...v0.2.0)

### Features

- **resolve:** say when unresolved imports come from an unloadable tsconfig ([#57](https://github.com/feds01/zenith/pull/57))
- **workspace:** `--tracked` scans only the files that git tracks ([#54](https://github.com/feds01/zenith/pull/54))
- **contracts:** read a baseline with merge conflicts as both sides ([#58](https://github.com/feds01/zenith/pull/58))
- **baseline:** `--prune` follows the files that git sees renamed ([#61](https://github.com/feds01/zenith/pull/61))
- **parse:** record the names each import takes, and what each file exports ([#63](https://github.com/feds01/zenith/pull/63))
- **graph:** a graph that follows imports through barrels ([#64](https://github.com/feds01/zenith/pull/64))
- **contracts:** `reexports = "transparent"` checks contracts through re-exports ([#65](https://github.com/feds01/zenith/pull/65))
- **npm:** ship the binary in a package per platform ([#62](https://github.com/feds01/zenith/pull/62))
- **check:** `zenith check <path>…` reports only what involves those files ([#66](https://github.com/feds01/zenith/pull/66))
- **pipeline:** cache what the parse stage finds in each file ([#48](https://github.com/feds01/zenith/pull/48))

### Fixes

- **graph:** nominate only cycle breakers that are on a cycle ([#53](https://github.com/feds01/zenith/pull/53))
- **cli:** warn about unresolved imports in every format of `check` ([#55](https://github.com/feds01/zenith/pull/55))
- **cli:** fail clearly when the project root has no source files ([#56](https://github.com/feds01/zenith/pull/56))
- **parse:** accept ES module syntax in `.cjs` files ([#59](https://github.com/feds01/zenith/pull/59))
- **contracts:** show how to prune a stale baseline entry in its annotation ([#60](https://github.com/feds01/zenith/pull/60))

### Documentation

- add zenith's logo, and use it as the book's favicon ([#50](https://github.com/feds01/zenith/pull/50))
- split the README, and show the logo in the book ([#52](https://github.com/feds01/zenith/pull/52))
- simplify the README and book introduction ([#68](https://github.com/feds01/zenith/pull/68))

## [0.1.0] - 2026-10-13

### Features

- **cli:** add the spec's commands and global options as stubs ([#4](https://github.com/feds01/zenith/pull/4))
- **utils:** add path helpers and the glob dialect ([#5](https://github.com/feds01/zenith/pull/5))
- **workspace:** add the file walker and workspace package discovery ([#7](https://github.com/feds01/zenith/pull/7))
- **graph:** add the module tree and import graph model ([#8](https://github.com/feds01/zenith/pull/8))
- **resolve:** add specifier resolution and its classification ([#9](https://github.com/feds01/zenith/pull/9))
- **graph:** add the grimp chain, layer and cycle breaker algorithms ([#12](https://github.com/feds01/zenith/pull/12))
- **config:** add the zenith.toml configuration and zenith schema ([#13](https://github.com/feds01/zenith/pull/13))
- **parse:** extract imports, directives and suppressions with oxc ([#11](https://github.com/feds01/zenith/pull/11))
- **bench:** add the zenith_benchmark crate and the CodSpeed workflow ([#10](https://github.com/feds01/zenith/pull/10))
- **pipeline:** run the stages that turn a project into its import graph ([#15](https://github.com/feds01/zenith/pull/15))
- **contracts:** resolve selectors into units ([#16](https://github.com/feds01/zenith/pull/16))
- **cli:** run graph, why and resolve on the pipeline ([#17](https://github.com/feds01/zenith/pull/17))
- **contracts:** compile and check forbidden and protected contracts ([#19](https://github.com/feds01/zenith/pull/19))
- **contracts:** check independence and layers contracts ([#20](https://github.com/feds01/zenith/pull/20))
- **contracts:** check acyclic and encapsulation contracts ([#21](https://github.com/feds01/zenith/pull/21))
- **bench:** benchmark a full check ([#28](https://github.com/feds01/zenith/pull/28))
- **cli:** check the contracts ([#22](https://github.com/feds01/zenith/pull/22))
- **contracts:** leave the imports that zenith-ignore suppresses out of contracts ([#24](https://github.com/feds01/zenith/pull/24))
- **cli:** record known violations with zenith baseline ([#25](https://github.com/feds01/zenith/pull/25))
- **just:** run a release build on a local checkout with just corpus ([#29](https://github.com/feds01/zenith/pull/29))
- **reporting:** add the json and github formats to zenith check ([#26](https://github.com/feds01/zenith/pull/26))
- **bench:** benchmark recording a baseline ([#38](https://github.com/feds01/zenith/pull/38))
- **cli:** write a starter configuration with zenith init ([#27](https://github.com/feds01/zenith/pull/27))
- **contracts:** boundaries that chains of imports stop at ([#47](https://github.com/feds01/zenith/pull/47))
- **just:** measure benchmarks with CodSpeed, in CI on demand or locally ([#43](https://github.com/feds01/zenith/pull/43))

### Performance

- **graph:** rule out most imports from edge sets without hashing ([#30](https://github.com/feds01/zenith/pull/30))
- **graph:** keep how a chain search reached each node in tables ([#31](https://github.com/feds01/zenith/pull/31))
- **utils:** make paths relative to the root as text ([#34](https://github.com/feds01/zenith/pull/34))
- **workspace:** sort the walk's paths by their bytes ([#35](https://github.com/feds01/zenith/pull/35))
- **cli:** allocate with mimalloc ([#36](https://github.com/feds01/zenith/pull/36))
- **graph:** keep import specifiers inline ([#37](https://github.com/feds01/zenith/pull/37))
- **contracts:** record baseline entries by their imports ([#39](https://github.com/feds01/zenith/pull/39))
- **pipeline:** keep imports of source files by the file's index ([#40](https://github.com/feds01/zenith/pull/40))
- **pipeline:** keep source files by their name only ([#41](https://github.com/feds01/zenith/pull/41))

### Documentation

- add the zenith build spec ([#3](https://github.com/feds01/zenith/pull/3))
- **spec:** mark milestones M1 to M3 done ([#44](https://github.com/feds01/zenith/pull/44))
- **book:** add a user guide ([#46](https://github.com/feds01/zenith/pull/46))

