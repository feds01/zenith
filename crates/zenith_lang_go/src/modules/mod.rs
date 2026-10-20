//! Reading what `go.mod` and `go.work` files say about a project's modules:
//! a module's path, what it requires and replaces, and the modules that a
//! workspace uses.
//!
//! Both readers are lenient, as `go` itself is not: a malformed line is
//! reported as a located [`ModError`] and skipped, and a directive that zenith
//! has no use for is skipped silently, so a file written for a newer toolchain
//! still yields what zenith needs.

mod go_mod;
mod go_work;

use std::{error::Error, fmt};

pub use go_mod::GoMod;
pub use go_work::GoWork;
use zenith_utils::{location::ByteRange, path::has_windows_drive};

/// A `require` of a module, at a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Require {
    /// The module path, such as `github.com/pkg/errors`.
    pub path: String,

    /// The version, such as `v0.9.1`.
    pub version: String,
}

/// A `replace` of a module, or of one version of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replace {
    /// The path of the module that is replaced.
    pub old: String,

    /// The version that is replaced, or `None` for every version.
    pub old_version: Option<String>,

    /// What replaces it.
    pub new: ReplaceTarget,
}

/// What replaces a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceTarget {
    /// Another module, at a version.
    Module {
        /// The module path.
        path: String,

        /// The version.
        version: String,
    },

    /// A directory on disk, as written: relative to the file's directory,
    /// such as `../errors`, or absolute.
    Path(String),
}

/// What a reader made of a file, and what it could not make sense of.
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

    /// What is wrong, in a sentence fragment such as `invalid syntax`.
    pub message: String,
}

impl fmt::Display for ModError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl Error for ModError {}

/// The text of a module path or a version as written, without the quotes of
/// a quoted string. `None` for an interpreted string with an escape in it,
/// which no real module path needs.
fn unquote(text: &str) -> Option<&str> {
    let quoted = |quote: char| text.strip_prefix(quote)?.strip_suffix(quote);
    if let Some(raw) = quoted('`') {
        return Some(raw);
    }
    match quoted('"') {
        Some(interpreted) => (!interpreted.contains('\\')).then_some(interpreted),
        None => Some(text),
    }
}

/// Whether a `replace` target is a directory rather than a module path, as Go
/// tells them apart: it is `.` or `..`, starts with `./`, `../` or `/`, or is
/// a Windows path, such as `..\shared` or `C:\work\shared`.
fn is_local_path(path: &str) -> bool {
    matches!(path, "." | "..")
        || path.starts_with(['/', '\\'])
        || ["./", "../", ".\\", "..\\"].iter().any(|prefix| path.starts_with(prefix))
        || has_windows_drive(path)
}
