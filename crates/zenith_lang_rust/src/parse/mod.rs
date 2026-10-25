//! The parser, which reads what Rust sources import with rust-analyzer's
//! parser, in the edition of each file's crate.
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

use std::{borrow::Cow, path::Path, sync::Arc};

use ra_ap_syntax::{SourceFile, TextRange};
use reader::Reader;
use thin_vec::ThinVec;
use zenith_diagnostics::message;
use zenith_lang::{ParsedFile, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

use crate::crates::Editions;

/// Parses Rust files into [`ParsedFile`]s, each in the edition of its crate.
pub(crate) struct Parser {
    /// The edition of each crate, which the parsers of every thread share.
    editions: Arc<Editions>,
}

impl Parser {
    /// A parser of Rust files, in the `editions` of their crates. It always
    /// records the names that imports take, and what each file exports, which
    /// resolving needs to tell an item from a module of the same name.
    pub(crate) fn new(editions: Arc<Editions>) -> Self {
        Self { editions }
    }

    /// What the Rust `source` of the file at `path` imports, with its
    /// suppression comments and syntax errors. A source of 4 GiB or more is
    /// not read, as offsets can't address it.
    pub(crate) fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile {
        if u32::try_from(source.len()).is_err() {
            let error = SyntaxError { message: Cow::Owned(message!(rust_source_too_large)) };
            let errors = ThinVec::from([Located::new(ByteRange::default(), error)]);
            return ParsedFile { errors, ..ParsedFile::default() };
        }
        let parse = SourceFile::parse(source, self.editions.of(path));
        let mut errors: ThinVec<_> = (parse.errors().into_iter())
            .map(|error| {
                let message = message(&error.to_string());
                Located::new(location(error.range()), SyntaxError { message })
            })
            .collect();
        errors.sort_by_key(|error| error.range);
        Reader::read(source, &parse.syntax_node(), errors)
    }
}

/// The range of `range` in the source, which is shorter than 4 GiB.
fn location(range: TextRange) -> ByteRange {
    ByteRange::new(range.start().into(), range.end().into())
}

/// rust-analyzer's `message`, with the kinds of token that it names, such as
/// `SEMICOLON` in "expected SEMICOLON", written as they are in code.
fn message(message: &str) -> Cow<'static, str> {
    let words: Vec<Cow<'_, str>> = message
        .split(' ')
        .map(|word| match token(word) {
            Some(token) => Cow::Owned(token),
            None => Cow::Borrowed(word),
        })
        .collect();
    Cow::Owned(words.join(" "))
}

/// The token of kind `kind`, such as `` `;` `` for `SEMICOLON`, or `None` if
/// `kind` is not the name of a kind of token.
fn token(kind: &str) -> Option<String> {
    if let Some(keyword) = kind.strip_suffix("_KW") {
        return Some(format!("`{}`", keyword.to_lowercase()));
    }
    let text = match kind {
        "SEMICOLON" => ";",
        "COMMA" => ",",
        "L_PAREN" => "(",
        "R_PAREN" => ")",
        "L_CURLY" => "{",
        "R_CURLY" => "}",
        "L_BRACK" => "[",
        "R_BRACK" => "]",
        "L_ANGLE" => "<",
        "R_ANGLE" => ">",
        "COLON" => ":",
        "COLON2" => "::",
        "EQ" => "=",
        "FAT_ARROW" => "=>",
        "THIN_ARROW" => "->",
        "DOT" => ".",
        "BANG" => "!",
        "POUND" => "#",
        "PIPE" => "|",
        "IDENT" => return Some("a name".to_owned()),
        _ => return None,
    };
    Some(format!("`{text}`"))
}
