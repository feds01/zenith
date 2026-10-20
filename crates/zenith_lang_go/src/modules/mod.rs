//! Reading `go.mod` and `go.work` files into typed structs.
//!
//! Both parsers are lenient, as `go` itself is not: a malformed line is
//! reported as a located [`ModError`] and skipped, and a directive that is not
//! known is skipped silently, so a file written for a newer toolchain still
//! yields what this one understands.

mod go_mod;
mod go_work;
mod lexer;
mod shared;

use std::{error::Error, fmt};

pub use go_mod::{GoMod, Require, Retract};
pub use go_work::GoWork;
pub use shared::{GoDebug, ModuleVersion, Replace, ReplaceTarget};
use zenith_utils::location::ByteRange;

/// What a parser made of a file, and what it could not make sense of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed<T> {
    /// Everything in the file that was understood.
    pub value: T,

    /// The lines that were malformed, in source order.
    pub errors: Vec<ModError>,
}

/// A malformed line in a `go.mod` or `go.work` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModError {
    /// The line the problem is on, counting from 1.
    pub line: usize,

    /// The bytes of the source that the problem is about.
    pub range: ByteRange,

    /// What is wrong, in a sentence fragment such as `unterminated string`.
    pub message: String,
}

impl fmt::Display for ModError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl Error for ModError {}
