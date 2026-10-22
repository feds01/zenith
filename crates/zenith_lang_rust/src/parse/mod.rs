//! The parser, which reads what Rust sources import with
//! [tree-sitter](https://tree-sitter.github.io)'s Rust grammar.
//!
//! An import is the leaf of a `use` declaration, written as its path with
//! `::`, an `extern crate`, or a path in code that starts with `crate`,
//! `self`, `super`, `::` or the name of a crate or module, such as
//! `zenith_lang_js::JS` used without a `use`. Everything that is not
//! syntax errors or comments is read from one walk of the syntax tree.

mod comments;
mod reader;

#[cfg(test)]
mod tests;

use std::{borrow::Cow, path::Path};

use reader::Reader;
use thin_vec::ThinVec;
use zenith_lang::{ParsedFile, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

/// Parses Rust files into [`ParsedFile`]s.
///
/// A parser holds tree-sitter's parser state, which it reuses between files,
/// so keep one per thread rather than making one per file.
pub(crate) struct Parser {
    /// The tree-sitter parser, set up with the Rust grammar.
    inner: tree_sitter::Parser,
}

impl Parser {
    /// A parser of Rust files.
    pub(crate) fn new() -> Self {
        let mut inner = tree_sitter::Parser::new();
        inner
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("the Rust grammar is built for this version of tree-sitter");
        Self { inner }
    }

    /// What the Rust `source` of the file at `path` imports, with its
    /// suppression comments and syntax errors. A source of 4 GiB or more is
    /// not read, as offsets can't address it.
    pub(crate) fn parse_source(&mut self, _path: &Path, source: &str) -> ParsedFile {
        if u32::try_from(source.len()).is_err() {
            let error = SyntaxError { message: Cow::Borrowed("the source is too large to read") };
            let errors = ThinVec::from([Located::new(ByteRange::default(), error)]);
            return ParsedFile { errors, ..ParsedFile::default() };
        }
        let tree = self
            .inner
            .parse(source, None)
            .expect("a parser with a language and no cancellation always parses");
        Reader::read(source, tree.root_node())
    }
}
