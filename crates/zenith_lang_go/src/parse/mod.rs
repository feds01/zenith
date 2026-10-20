//! Reads what a Go source file imports.
//!
//! A [`Parser`] parses the source with
//! [tree-sitter](https://tree-sitter.github.io)'s Go grammar, then reads the
//! comments and the package clause at the top of the file, and the import
//! declarations, which Go requires to come straight after the package clause.
//!
//! ```
//! use zenith_lang_go::parse::{ImportName, Parser};
//!
//! let source = r#"package main
//!
//! import (
//!     "fmt"
//!     log "github.com/acme/logging" // the house logger
//!     _ "embed"
//! )
//! "#;
//! let file = Parser::new().parse(source);
//! assert_eq!(file.package.unwrap().value, "main");
//! assert!(file.errors.is_empty());
//!
//! let paths: Vec<_> = file.imports.iter().map(|import| import.path.as_str()).collect();
//! assert_eq!(paths, ["fmt", "github.com/acme/logging", "embed"]);
//! assert_eq!(file.imports[1].name, ImportName::Alias("log".to_owned()));
//! assert_eq!(file.imports[2].name, ImportName::Blank);
//! ```
//!
//! The file is read as written: build constraints are reported but not
//! evaluated. Malformed input never panics. Each syntax error at the top of the
//! file is recorded in [`GoFile::errors`], and the imports around it are still
//! read.

mod reader;

#[cfg(test)]
mod tests;

use std::fmt;

use zenith_utils::location::{ByteRange, Located};

/// Parses Go source files into [`GoFile`]s.
///
/// A parser holds tree-sitter's parser state, which it reuses between files,
/// so keep one per thread rather than making one per file.
pub struct Parser {
    /// The tree-sitter parser, set up with the Go grammar.
    inner: tree_sitter::Parser,
}

impl Parser {
    /// A parser of Go files.
    pub fn new() -> Self {
        let mut inner = tree_sitter::Parser::new();
        inner
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .expect("the Go grammar is built for this version of tree-sitter");
        Self { inner }
    }

    /// Read the package clause, the import declarations and the comments in
    /// front of them from the Go `source` of one file.
    ///
    /// All offsets are bytes into `source`, which must be shorter than 4 GiB. A
    /// longer source is not read, and reports
    /// [`GoSyntaxError::SourceTooLarge`].
    pub fn parse(&mut self, source: &str) -> GoFile {
        if u32::try_from(source.len()).is_err() {
            let mut file = GoFile::default();
            file.errors.push(Located::new(ByteRange::default(), GoSyntaxError::SourceTooLarge));
            return file;
        }
        let tree = self
            .inner
            .parse(source, None)
            .expect("a parser with a language and no cancellation always parses");
        reader::read(source, tree.root_node())
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

/// What a Go source file imports, with where in the file each thing is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoFile {
    /// The name in the package clause, located by the name. `None` if the file
    /// has no package clause, or no name in it.
    pub package: Option<Located<String>>,

    /// The expression of the file's `//go:build` constraint, such as
    /// `linux && !cgo`, located by the expression. `None` if the file has no
    /// constraint. Only a constraint in front of the package clause counts. The
    /// legacy `// +build` lines are ignored.
    pub build_constraint: Option<Located<String>>,

    /// The imports of the file, in source order. An import that could not be
    /// read, such as one with an unterminated path or an escape in its path,
    /// is left out, and reported in [`errors`](Self::errors).
    pub imports: Vec<GoImport>,

    /// The comments from the start of the file to the end of its imports, in
    /// source order. That includes a comment after the last import on its line,
    /// but not the comments after that.
    ///
    /// Comments are what the `zenith-ignore` suppressions are read from, and
    /// the cgo preamble of an `import "C"` is the comment in front of it.
    pub comments: Vec<GoComment>,

    /// The syntax errors found, in the order they were found.
    pub errors: Vec<Located<GoSyntaxError>>,
}

/// One import spec: the `"path"` in `import "path"`, or a line of an
/// `import ( ... )` group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoImport {
    /// The import path, between its quotes, such as `fmt` or
    /// `github.com/acme/logging/v2`. For `import "C"`, a cgo import, it is
    /// `C`.
    pub path: String,

    /// The whole spec, from its name, or else its path, to the end of its
    /// path. It does not include the `import` keyword, even when the spec is
    /// not in a group.
    pub range: ByteRange,

    /// The path as written, with its quotes.
    pub path_range: ByteRange,

    /// How the import is named.
    pub name: ImportName,
}

/// The name an import is bound to in the importing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportName {
    /// No name, so the name is the one in the package clause of the imported
    /// package.
    Default,

    /// A name written in front of the path: `log "github.com/acme/logging"`.
    Alias(String),

    /// `_`: the package is imported only for its side effects.
    Blank,

    /// `.`: the exported names of the package are in the scope of the file.
    Dot,
}

/// A comment, found by its range. Its text is that range of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoComment {
    /// The whole comment, from its `//` or `/*` to the end of its line or its
    /// `*/`. The `\r` of a `\r\n` line ending is not part of it.
    pub range: ByteRange,

    /// Whether it is a `/* */` comment, rather than a `//` comment.
    pub block: bool,
}

/// What is wrong in a syntax error of a Go file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GoSyntaxError {
    /// The source is 4 GiB or longer, which is more than offsets can address.
    SourceTooLarge,

    /// The file does not start with a package clause.
    MissingPackageClause,

    /// Code that is not valid Go, such as a stray token, an unterminated
    /// string or an unterminated comment.
    Invalid,

    /// Something that Go requires is missing, such as the `)` of an import
    /// group. It holds the name of what is missing, in the Go grammar of
    /// tree-sitter: a token, such as `)`, or a kind of node, such as
    /// `package_identifier`.
    Missing(String),

    /// An import path has an escape sequence in it, such as `"fmt\x41"`,
    /// which zenith does not decode. Real import paths never need one.
    EscapedImportPath,

    /// An import path is the empty string.
    EmptyImportPath,
}

impl fmt::Display for GoSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceTooLarge => f.write_str("the source is too large to read"),
            Self::MissingPackageClause => f.write_str("expected the package clause"),
            Self::Invalid => f.write_str("invalid syntax"),
            // A kind of node is a name in snake case, and a token is anything else.
            Self::Missing(kind) if kind.chars().all(|c| c.is_ascii_lowercase() || c == '_') => {
                write!(f, "expected {}", kind.replace('_', " "))
            }
            Self::Missing(token) => write!(f, "expected `{token}`"),
            Self::EscapedImportPath => f.write_str("escapes in import paths are not supported"),
            Self::EmptyImportPath => f.write_str("the import path is empty"),
        }
    }
}
