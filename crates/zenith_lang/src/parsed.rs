//! What zenith learns from one source file.

use thin_vec::ThinVec;
use zenith_graph::{Exports, ImportDetail, ImportKind};
use zenith_utils::location::Located;

use crate::{Suppression, SyntaxError};

/// What zenith learns from one source file.
///
/// A language's parser produces it: the imports of the file, its directive
/// prologue (e.g. `"use client"`), its `zenith-ignore` suppression comments,
/// the imports whose specifier is not a literal, and any syntax errors.
/// Everything is [`Located`] by its byte range in the file.
///
/// Every list but the imports is empty in most files, so those are
/// [`ThinVec`]s, which take one pointer each rather than three.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedFile {
    /// The imports, in source order, before their specifiers are resolved.
    pub imports: Vec<ImportDetail>,

    /// What the file exports, with the source of each re-export as the index
    /// of its import in [`imports`](Self::imports). Nothing, unless the
    /// parser records names.
    pub exports: Exports<u32>,

    /// The imports whose specifier is not a literal, e.g. JavaScript's
    /// `import(path)` or `` require(`./${name}`) ``, or a Terraform module
    /// source made from a template, in source order. Each is located by the
    /// call or the expression, with the kind the import would have. They
    /// can't be resolved, so they are not in [`imports`](Self::imports).
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
