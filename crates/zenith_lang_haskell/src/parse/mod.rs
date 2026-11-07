//! Reads what a Haskell source file imports.
//!
//! A [`Parser`] reads a source in three steps, each from a chapter of the
//! [Haskell 2010 Report](https://www.haskell.org/onlinereport/haskell2010/):
//!
//! 1. The lexer splits it into its lexemes (chapter 2), skipping whitespace,
//!    the pragmas other than `{-# SOURCE #-}`, and the lines of the C
//!    preprocessor, and keeping the comments.
//! 2. The layout rule groups the lexemes into the module header and the
//!    top-level declarations after it (section 10.3).
//! 3. The grammar reads the header, and each import, from its lexemes (chapter
//!    5), with the extensions of GHC that change them.
//!
//! Imports come before every other declaration, so the first declaration
//! that isn't one ends the reading, and nothing after it is lexed.
//!
//! The lexer and the grammar are built with the parser combinators of
//! [winnow](https://docs.rs/winnow), which read text and slices of tokens
//! alike: the lexer reads the source, and the grammar the lexemes of a
//! declaration, without allocating, and a parser that fails says what it
//! expected. A grammar generator, such as pest or LALRPOP, would still need
//! the layout rule's own pass over the lexemes, and a lexer generator, such
//! as logos, describes lexemes with regular expressions.
//!
//! ```
//! use zenith_lang_haskell::parse::{Dialect, Parser};
//!
//! let source = r#"module Acme.Api (serve) where
//!
//! import Data.Map.Strict (Map)
//! import qualified Data.Text as T
//! import "aeson" Data.Aeson -- the JSON codecs
//!
//! serve :: IO ()
//! serve = pure ()
//! "#;
//! let file = Parser::new().parse(source, Dialect::PLAIN);
//! assert_eq!(file.module.unwrap().value, "Acme.Api");
//! assert!(file.errors.is_empty());
//!
//! let modules: Vec<_> = file.imports.iter().map(|import| import.module.as_str()).collect();
//! assert_eq!(modules, ["Data.Map.Strict", "Data.Text", "Data.Aeson"]);
//! assert!(file.imports[1].qualified);
//! assert_eq!(file.imports[2].package.as_deref(), Some("aeson"));
//! ```
//!
//! The C preprocessor is not run. Each declaration is read as one build of
//! the file would read it, from one branch of each conditional, so the
//! imports of every branch are read, and a header or an import list that each
//! branch writes its own way is read once, from the first. Malformed input
//! never panics. Each syntax error in the header or in an import is recorded
//! in [`HaskellFile::errors`], and the imports around it are still read.

mod error;
mod grammar;
mod lang;
mod layout;
mod lexer;
mod literate;
mod reader;

#[cfg(test)]
mod tests;

use std::path::Path;

pub use error::{Expected, HaskellSyntaxError};
pub(crate) use lang::Specifier;
use reader::Reader;
use zenith_utils::location::{ByteRange, Located};

/// Parses Haskell files into [`HaskellFile`]s.
///
/// A parser keeps the buffer that it reads literate files into, which it
/// reuses between files, so keep one per thread rather than making one per
/// file.
#[derive(Debug, Default)]
pub struct Parser {
    /// The code of the last literate file read, as plain Haskell.
    code: String,
}

impl Parser {
    /// A parser of Haskell files.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the module header, the imports and the comments in front of them
    /// from the `source` of one file, written in `dialect`.
    ///
    /// All offsets are bytes into `source`, which must be shorter than 4 GiB. A
    /// longer source is not read, and reports
    /// [`HaskellSyntaxError::SourceTooLarge`].
    pub fn parse(&mut self, source: &str, dialect: Dialect) -> HaskellFile {
        if u32::try_from(source.len()).is_err() {
            let mut file = HaskellFile::default();
            file.errors
                .push(Located::new(ByteRange::default(), HaskellSyntaxError::SourceTooLarge));
            return file;
        }
        let keyword = dialect.header_keyword();
        match dialect.literate {
            false => Reader::new(source, keyword).read(),
            true => {
                literate::unlit(source, &mut self.code);
                Reader::new(&self.code, keyword).read()
            }
        }
    }
}

/// How a file is written: which of it is code, and what its header starts
/// with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dialect {
    /// Whether it is literate Haskell, as a `.lhs` file is, whose code is in
    /// the lines that start with `>`, or between `\begin{code}` and
    /// `\end{code}`.
    pub literate: bool,

    /// Whether it is a signature of Backpack's, as a `.hsig` file is, whose
    /// header starts with `signature` rather than `module`.
    pub signature: bool,
}

impl Dialect {
    /// Plain Haskell, as in a `.hs` file, which is all code.
    pub const PLAIN: Self = Self { literate: false, signature: false };

    /// Literate Haskell, as in a `.lhs` file.
    pub const LITERATE: Self = Self { literate: true, signature: false };

    /// The dialect of the file at `path`, by its extension: literate for a
    /// `.lhs` file, its boot file's `.lhs-boot` and its signature's `.lhsig`,
    /// and a signature for a `.hsig` or a `.lhsig` file.
    pub fn of(path: &Path) -> Self {
        let extension =
            path.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
        Self {
            literate: matches!(extension, "lhs" | "lhs-boot" | "lhsig"),
            signature: matches!(extension, "hsig" | "lhsig"),
        }
    }

    /// The keyword that the header of a file in it starts with.
    fn header_keyword(self) -> &'static str {
        match self.signature {
            true => "signature",
            false => "module",
        }
    }
}

/// What a Haskell file imports, with where in the file each thing is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HaskellFile {
    /// The name of the module in its header, such as `Acme.Api`, located by
    /// the name. `None` if the file has no header, which makes it the module
    /// `Main`, or if its header can't be read.
    pub module: Option<Located<String>>,

    /// The imports of the file, in source order. An import that can't be
    /// read is left out, and reported in [`errors`](Self::errors).
    pub imports: Vec<Import>,

    /// The comments from the start of the file to the end of its imports, in
    /// source order. That includes the comments after the last import on the
    /// line where it ends, but not the comments after that.
    ///
    /// Comments are what the `zenith-ignore` suppressions are read from.
    pub comments: Vec<HaskellComment>,

    /// The syntax errors found, in the order they were found.
    pub errors: Vec<Located<HaskellSyntaxError>>,
}

/// One import declaration: `import qualified Data.Map as Map`, say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The name of the module, such as `Data.Map`.
    pub module: String,

    /// The whole declaration, from its `import` to its end.
    pub range: ByteRange,

    /// The name of the module, as written.
    pub module_range: ByteRange,

    /// The package that the import names the module's package with, between
    /// its quotes, as `aeson` in `import "aeson" Data.Aeson`. GHC's
    /// `PackageImports` extension allows it.
    pub package: Option<String>,

    /// Whether the import is of the module's boot file, as
    /// `import {-# SOURCE #-} Acme.Types` is, which breaks a cycle of
    /// imports.
    pub source: bool,

    /// The level of Template Haskell that the import is for, if it names
    /// one, as GHC's `ExplicitLevelImports` extension allows.
    pub level: Option<Level>,

    /// Whether the import is `safe`, which GHC's Safe Haskell checks.
    pub safe: bool,

    /// Whether the module's names are only in scope qualified, by `qualified`
    /// in front of the module's name or, as GHC's `ImportQualifiedPost`
    /// extension allows, after it.
    pub qualified: bool,

    /// The name that the module is known by in the file, after `as`.
    pub alias: Option<String>,

    /// The names that the import takes, or hides, if it lists them.
    pub list: Option<ImportList>,
}

/// The level of Template Haskell that an import brings its module's names in
/// at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `splice`: for the splices of the importing module, which run as it
    /// compiles.
    Splice,

    /// `quote`: for the quotes of the importing module.
    Quote,
}

/// The list of an import: the names in parentheses after its module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportList {
    /// Whether the list follows `hiding`, so its names are all that the
    /// import leaves out, rather than all that it takes.
    pub hiding: bool,

    /// The names of the list, each located by its name. A type or a class is
    /// named without the constructors, fields or methods after it, an
    /// operator without its parentheses, as `<>` for `(<>)`, and every name of
    /// a namespace, as `type ..` takes, as `..`.
    pub names: Vec<Located<String>>,
}

/// A comment, found by its range. Its text is that range of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HaskellComment {
    /// The whole comment, from its `--` to the end of its line, or from its
    /// `{-` to its `-}`. The `\r` of a `\r\n` line ending is not part of it.
    pub range: ByteRange,

    /// Whether it is a `{- -}` comment, rather than a `--` comment.
    pub block: bool,
}
