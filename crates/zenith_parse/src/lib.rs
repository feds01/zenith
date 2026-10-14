//! Import extraction for JavaScript and TypeScript files.
//!
//! A [`Parser`] parses source text with [oxc](https://oxc.rs), then walks the
//! syntax tree for every form that imports a module, classifying each by its
//! [`ImportKind`]. Alongside the imports, the [`ParsedFile`] holds the file's
//! directive prologue (e.g. `"use client"`), its `zenith-ignore` suppression
//! comments, the dynamic imports and `require` calls whose specifier is not a
//! literal, and any syntax errors. Everything is [`Located`] by its byte
//! range in the file. A parser that records names also finds the names that
//! each import takes, what the file exports, and whether it only passes
//! re-exports through.

mod error;
mod exports;
mod parser;
mod suppressions;
mod visitor;

pub use error::SyntaxError;
pub use parser::{ParseOptions, Parser};
pub use suppressions::Suppression;
use thin_vec::ThinVec;
use zenith_graph::{Exports, ImportDetail, ImportKind};
use zenith_utils::location::Located;

/// What zenith learns from one source file.
///
/// Every list but the imports is empty in most files, so those are
/// [`ThinVec`]s, which take one pointer each rather than three.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedFile {
    /// The imports, in source order, before their specifiers are resolved.
    pub imports: Vec<ImportDetail>,

    /// What the file exports, with the source of each re-export as the index
    /// of its import in [`imports`](Self::imports). Nothing, unless the
    /// parser records names (see [`ParseOptions::names`]).
    pub exports: Exports<u32>,

    /// The dynamic imports and `require` calls whose specifier is not a
    /// string literal or a template literal without substitutions, e.g.
    /// `import(path)` or `` require(`./${name}`) ``, in source order. Each is
    /// located by the call, and is [`ImportKind::Dynamic`] or
    /// [`ImportKind::Require`]. They can't be resolved, so they are not in
    /// [`imports`](Self::imports).
    pub non_literal_imports: ThinVec<Located<ImportKind>>,

    /// The directive prologue, e.g. `["use client"]`: the string-literal
    /// statements at the very top of the file, each as written between its
    /// quotes.
    pub directives: ThinVec<String>,

    /// The `zenith-ignore` comments, in source order.
    pub suppressions: ThinVec<Located<Suppression>>,

    /// The syntax errors, in source order. The parser recovers from some
    /// errors and keeps what it found. When it can't, the rest of the result
    /// is empty.
    pub errors: ThinVec<Located<SyntaxError>>,
}
