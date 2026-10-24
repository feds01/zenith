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

Every import is a `value` import. Each `use` declaration imports each of its
leaves, so `use a::{b, c::d}` imports `a::b` and `a::c::d`, and a glob imports
the path before it. So does an `extern crate`, and so does a path in code that
starts with `crate`, `self`, `super`, `::` or the name of a crate or module,
such as `zenith_lang_js::JS` written without a `use`. A `use` path that starts
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
| the standard library | `rust:std` | `std`, `core`, `alloc`, `proc_macro` and `test`. |

An inline `mod a { }` block is part of its file, so a path into it, `use`d or
not, imports nothing. `#[path]` attributes are not followed, and a file
doesn't import itself. A raw identifier names what it spells: `r#false` is the
module `false`. A path that starts with the name of a tool, such as
`clippy::too_many_lines` in an `#[allow]` or `rustfmt::skip`, is the tool's
attribute or lint, and imports nothing unless a crate or module has the name.
