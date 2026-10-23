//! The parser, which reads what Python sources import with
//! [Ruff](https://github.com/astral-sh/ruff)'s parser.
//!
//! An import is each module of an `import` statement, and each name of a
//! `from` import, written after its module with `.`, so that the resolver can
//! tell a submodule from a name in the module. A lazy import, such as
//! `lazy import a`, is an import like any other. Imports in an
//! `if TYPE_CHECKING:` block are type imports, and a call of
//! `importlib.import_module` or `__import__` is a dynamic one. Everything
//! but the comments is read from one walk of the syntax tree, and the
//! comments from its tokens.

mod comments;
mod reader;

#[cfg(test)]
mod tests;

use std::{borrow::Cow, path::Path};

use reader::Reader;
use ruff_python_ast::PySourceType;
use ruff_python_parser::parse_unchecked_source;
use thin_vec::ThinVec;
use zenith_lang::{ParsedFile, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

/// Parses Python files into [`ParsedFile`]s.
///
/// Ruff's parser keeps nothing between files, so a parser holds no state, and
/// is made per thread only as the parsers of the other languages are.
#[derive(Debug, Default)]
pub struct Parser;

impl Parser {
    /// A parser of Python files.
    pub fn new() -> Self {
        Self
    }

    /// What the Python `source` of the file at `path`, a module or a stub by
    /// its extension, imports, with its suppression comments and syntax
    /// errors. A source of 4 GiB or more is not read, as offsets can't
    /// address it.
    pub fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile {
        if u32::try_from(source.len()).is_err() {
            let error = SyntaxError { message: Cow::Borrowed("the source is too large to read") };
            let errors = ThinVec::from([Located::new(ByteRange::default(), error)]);
            return ParsedFile { errors, ..ParsedFile::default() };
        }
        let parsed = parse_unchecked_source(source, PySourceType::from(path));
        Reader::read(source, &parsed)
    }
}
