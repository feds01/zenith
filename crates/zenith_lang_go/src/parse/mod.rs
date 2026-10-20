//! Reads what a Go source file imports.
//!
//! Go requires the import declarations to come straight after the package
//! clause, so this is not a Go parser: it scans the comments, the package
//! clause and the import declarations, and stops at the first other top-level
//! token.
//!
//! ```
//! use zenith_lang_go::parse::{ImportName, parse};
//!
//! let source = r#"package main
//!
//! import (
//!     "fmt"
//!     log "github.com/acme/logging" // the house logger
//!     _ "embed"
//! )
//! "#;
//! let file = parse(source);
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
//! evaluated, and nothing is checked beyond what is needed to find the
//! imports. Malformed input never panics. It is recovered from where
//! possible, and each problem found is recorded in [`GoFile::errors`].

mod scanner;
mod string;

#[cfg(test)]
mod tests;

use std::fmt;

use zenith_utils::location::{ByteRange, Located};

/// Read the package clause, the import declarations and the comments in front
/// of them from the Go `source` of one file.
///
/// All offsets are bytes into `source`, which must be shorter than 4 GiB. A
/// longer source is not read, and reports [`GoSyntaxError::SourceTooLarge`].
pub fn parse(source: &str) -> GoFile {
    if u32::try_from(source.len()).is_err() {
        let mut file = GoFile::default();
        file.errors.push(Located::new(ByteRange::default(), GoSyntaxError::SourceTooLarge));
        return file;
    }
    scanner::Scanner::new(source).scan()
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
    /// read, such as one with an unterminated path, is left out, and reported
    /// in [`errors`](Self::errors).
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
    /// The import path, with the escapes of an interpreted string decoded,
    /// such as `fmt` or `github.com/acme/logging/v2`. For `import "C"`, a
    /// cgo import, it is `C`.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GoSyntaxError {
    /// The source is 4 GiB or longer, which is more than offsets can address.
    SourceTooLarge,

    /// The file does not start with a package clause.
    MissingPackageClause,

    /// The `package` keyword has no name after it.
    MissingPackageName,

    /// A `/* */` comment is never closed. It runs to the end of the file.
    UnterminatedComment,

    /// A string is never closed. An interpreted string must end on the line it
    /// starts on.
    UnterminatedString,

    /// An escape sequence in an interpreted string is not one of Go's, or has
    /// too few digits, or a value that is not allowed.
    InvalidEscape,

    /// An interpreted string with `\x` or octal escapes is not valid UTF-8
    /// once its escapes are decoded.
    InvalidUtf8,

    /// An import spec has no path, where a string was expected.
    MissingImportPath,

    /// An import path is the empty string.
    EmptyImportPath,

    /// An `import (` group is never closed, because the file ends, or a
    /// `func`, `var`, `type` or `const` declaration starts, before its `)`.
    UnterminatedImportGroup,

    /// Something that is not an import spec is in an import group.
    UnexpectedToken,
}

impl fmt::Display for GoSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SourceTooLarge => "the source is too large to read",
            Self::MissingPackageClause => "expected the package clause",
            Self::MissingPackageName => "expected the package name",
            Self::UnterminatedComment => "comment not terminated",
            Self::UnterminatedString => "string literal not terminated",
            Self::InvalidEscape => "invalid escape sequence in string literal",
            Self::InvalidUtf8 => "string literal is not valid UTF-8",
            Self::MissingImportPath => "expected the import path, a string literal",
            Self::EmptyImportPath => "the import path is empty",
            Self::UnterminatedImportGroup => "import group not terminated, expected `)`",
            Self::UnexpectedToken => "expected an import spec",
        })
    }
}
