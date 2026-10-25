# Rust

```toml
[languages.rust]           # off unless the table is written
include = ["**/*.rs"]      # the default
ignore_unresolved = ["generated::*"]   # paths not to warn about
```

| Option | |
|---|---|
| `include` | Globs of the files to scan. |
| `ignore_unresolved` | Name patterns of the paths that are expected not to resolve, so are not reported as unresolved, such as a module that a macro declares. |

## Packages

Every Cargo package of the project is a crate, which `pkg:` selects by its
package name, so `pkg:zenith_lang_*` selects the crates whose names start with
`zenith_lang_`. zenith asks Cargo for the crates, with
`cargo metadata --no-deps --offline`, which reads the manifests but resolves no
versions and never touches the network. Without Cargo, or when it fails,
zenith reads the `Cargo.toml` files itself and finds each crate's targets the
way Cargo does.

Each target of a crate is the root of a module tree: its library, its binaries,
its tests, benchmarks and examples, and its build script. The other targets of
a crate use its library by its name.

## Imports

An import that only tests compile is a `test` import, and any other is a
`value` import. Tests compile the code under a `#[cfg]` whose predicate only
holds in tests, such as `#[cfg(test)]` or `#[cfg(all(test, unix))]`, and the
functions with a test attribute, such as `#[test]` or `#[tokio::test]`. They
alone compile a module declared under such a `#[cfg]`, as in
`#[cfg(test)] mod tests;`, with every module in it, a file that starts with
`#![cfg(test)]`, and the tests, benchmarks and examples of a crate. A
dev-dependency can only be used by that code, so its imports are `test`
imports too. `exclude_kinds = ["test"]` leaves them out of a contract, so that
the helpers of the tests, which import the modules they test, don't make
cycles with them:

```toml
[contracts.modules-are-acyclic]
type = "acyclic"
modules = ["crates/*/src/*"]
exclude_kinds = ["test"]
```

Each `use` declaration imports each of its
leaves, so `use a::{b, c::d}` imports `a::b` and `a::c::d`, and a glob imports
the path before it. So does an `extern crate`, and so does a path in code that
starts with `crate`, `self`, `super`, `::` or the name of a crate or module,
such as `zenith_lang_js::JS` written without a `use`. A path in code imports
its modules and the first item after them that starts with a capital, which
may be a type: `std::time::Instant::now()` imports `std::time::Instant`. A `use` path that starts
with a name that another `use` or an `extern crate` brought into scope imports
what the name stands for: after `use smoltcp::socket::tcp;`, `use tcp::State;`
imports `smoltcp::socket::tcp::State`. `mod a;` declares a module but doesn't
import it. [`zenith-ignore` comments](../suppressions.md) work as `//` or
`/* */` comments.

The tokens of a macro call or an attribute aren't parsed as code, so zenith
reads the paths in them: `vec![crate::a::b()]` imports `crate::a::b`. A path
that starts at a macro variable, such as `$crate::a` or `$module::a`, or at an
interpolation, which macros like `quote!` write as `#krate::a`, imports
nothing. Nor does a path in the body of `quote!`, `quote_spanned!`,
`parse_quote!` or `parse_quote_spanned!`, whatever path names the macro: the
body is code that a procedural macro generates for the crates that use it, so
its paths are theirs, not imports of the macro's crate.

Files are parsed with [rust-analyzer](https://rust-analyzer.github.io)'s
parser, in the edition of their crate, which Cargo or the crate's
`Cargo.toml` gives (and Rust 2015 if it gives none). A file in no crate is
parsed in the latest edition.

## Resolution

A path imports the file of the last module on it: `a.rs` or `a/mod.rs` for the
module `a`, in the module tree of the importing file's target. The rest of the
path is items in that file. The file is spelt as the module is, even where
names ignore case, as they do by default on macOS and Windows, so `crate::Config`
is the type `Config` in the crate root, not the module in `config.rs`. A crate's own dependencies, from its
`[dependencies]`, `[dev-dependencies]` and `[build-dependencies]`, including
renamed ones and those that `workspace = true` takes from the workspace, are
the crates that its code may name.

| Imported | Node | |
|---|---|---|
| a module of the project | `crates/api/src/store.rs` | Its file. A dependency that is a crate of the project resolves into the module tree of its library. |
| another crate | `crate:serde` | Named by its package. |
| the standard library | `rust:std::thread` | A path into `std`, `core`, `alloc`, `proc_macro` or `test`, up to its first item that starts with a capital: `std::thread::spawn` is `rust:std::thread::spawn`, and `std::time::Instant::now` is `rust:std::time::Instant`. |

A selector of the standard library matches a path and everything under it, so
`rust:std` matches every path into `std`, `rust:std::{thread,fs}` the paths into
its `thread` and `fs` modules, and `rust:std::time::Instant` the type. A core
that builds for WebAssembly, where threads, the clock and the file system
compile but fail at run time, can forbid them:

```toml
[contracts.core-builds-for-the-browser]
type = "forbidden"
from = ["pkg:core"]
to = ["rust:std::{thread,fs,process,net}", "rust:std::time::Instant"]
```

A path to an item that the crate root re-exports, such as `use crate::Layout`
after `pub use layout::Layout;` in `lib.rs`, imports `lib.rs`, which imports
the module it re-exports from. A chain through the root then joins every
module that imports through it to every module that it re-exports, so a
contract that follows chains over the modules of a crate sees them all
depend on each other, unless it sees through [re-exports](#re-exports).

An inline `mod a { }` block is part of its file, so a path into it, `use`d or
not, imports nothing. `#[path]` attributes are not followed, and a file
doesn't import itself. A raw identifier names what it spells: `r#false` is the
module `false`. A path that starts with the name of a tool, such as
`clippy::too_many_lines` in an `#[allow]` or `rustfmt::skip`, is the tool's
attribute or lint, and imports nothing unless a crate or module has the name.

## Re-exports

A Rust module runs no code when it is imported, so a contract with
[`reexports = "transparent"`](../concepts.md#re-exports) sees an import of an
item as an import of the module that defines it, through the `use`
declarations of the modules on the way, globs included: with
`pub use layout::Layout;` in `lib.rs`, `use crate::Layout` imports
`layout.rs`. A `use` re-exports what it brings into scope to the modules that
may see it, whatever its visibility. The item that a path takes is its first
segment after the crate or module that starts with a capital, such as `Shape`
in `crate::Shape::Circle`, or else its last, such as `write` in
`crate::write`. An import of a whole module, such as a glob or
`use crate::units::{self}`, still imports its file, and so does an item that
zenith can't follow, such as one that may come from another crate through a
glob.

```toml
[contracts.typeset-modules]
type = "layers"
containers = ["crates/typeset/src"]
layers = ["render", "document", "layout", "units"]
reexports = "transparent"
```

A module that also has code of its own, such as a crate root with functions,
is followed all the same, for the items it doesn't define. Without
re-exports seen through, making the crate roots
[boundaries](../contracts/overview.md#boundaries) of a contract, as in
`boundaries = ["crates/*/src/lib.rs"]`, stops chains at them instead.
