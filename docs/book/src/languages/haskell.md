# Haskell

```toml
[languages.haskell]                 # off unless the table is written
include = ["services/**/*.hs"]      # every Haskell source file unless given
ignore_unresolved = ["Generated.*"] # module names not to warn about
```

| Option | |
|---|---|
| `include` | Globs of the files to scan. By default, every file with a Haskell extension: `.hs`, `.lhs`, `.hs-boot`, `.lhs-boot`, `.hsig`, `.lhsig` and `.hsc`. |
| `ignore_unresolved` | Name patterns of the module names that are expected not to resolve, so are not reported as unresolved. |

## Packages

Every `.cabal` file is a package, which `pkg:` selects by the name it gives,
so `pkg:acme-*` selects the packages whose names start with `acme-`. A
package is made of its components, its libraries, executables, test suites
and benchmarks, which each say where their modules are, in
`hs-source-dirs`, and which libraries they depend on, in `build-depends`.
Their conditional branches, `common` stanzas and `import:` fields all count.
A package that hpack describes in a `package.yaml` is read from the `.cabal`
file that hpack or stack writes from it.

## Builds

Which package an installed module comes from is something only a build of
the project knows, as GHC does. So zenith reads what a build installed: the
plan of a cabal build, in `dist-newstyle`, and the package databases that it
names, or those of a stack build, in `.stack-work`. It looks for a build
where a `cabal.project` or `stack.yaml` is, in each package's directory, and
at the project root, and asks `cabal` or `stack` where its databases are,
with `cabal path`, from cabal 3.12, or `stack path`. A build whose tool is
not on the `PATH`, as it may not be for an editor started outside a shell,
can't be read, with a warning. Neither directory is ever scanned.

A project that hasn't been built, or a package that no build is of, sees the
libraries that come with the compiler, such as `base` and `containers`, which
it asks `ghc` for. Its other imports don't resolve, unless they name their
package, until the project is built with `cabal build` or `stack build`.

## Imports

zenith reads the module header and the imports of each file with a parser
of its own. It follows the layout rule, reads literate files, both with Bird
tracks and with `\begin{code}` blocks, and takes each import from one branch
of the CPP conditionals around it: the branch that it starts in. `#if 0`
blocks are skipped.

| Import | Kind |
|---|---|
| any import | `value` |
| any import of a file that only test suites and benchmarks compile | `test` |

An import that names its package, such as `import "text" Data.Text`, looks
only in that package, and an import of a boot file, such as
`import {-# SOURCE #-} Acme.Types`, is an import of the module's `.hs-boot`
file. [`zenith-ignore` comments](../suppressions.md) work among a file's
imports, as `--` or `{- -}` comments.

## Resolution

A module is looked for as GHC looks for it when it builds the component
that compiles the importing file: among the component's own modules, then
among those of the libraries that it depends on, seen through its `mixins`,
and the modules that those libraries re-export. A component compiles its
`main-is` file, and the files in its source directories whose paths are
module names, as `Acme/Core.hs` is, so a library whose source directory is
its package's doesn't compile the test suite in `test`. A file that no
component compiles, such as a script, or a module of a library that the component
forgot to depend on, is looked for in every library of the project, then in
every one that the build installed.

| Imported | Node | |
|---|---|---|
| a module of the project | `core/src/Acme/Core.hs` | Its file. A module that a preprocessor makes is its source: its `.hsc`, `.chs`, `.x` or `.y` file, for `hsc2hs`, c2hs, Alex or Happy, though only `.hsc` files are scanned. |
| a module of an installed library | `hackage:containers` | Named by the library's package, so `Data.Map` is `hackage:containers`. A module that `base` re-exports is `hackage:base`. |
| a module of a package that the import names, which no build installed | `hackage:text` | Named by the package. |

A module that Cabal writes while it builds the package, such as
`Paths_acme_core`, has no file, so it is left out. A module that more than one
library has is ambiguous, as it is for GHC, and doesn't resolve until the
import names its package. Names are compared exactly, even where the file
system ignores case. A file doesn't import itself.
