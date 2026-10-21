//! Turning a Go import path into what it refers to: a package of the project,
//! of the standard library, or of a dependency.

mod discover;
mod go_modules;
mod heuristic;

pub use discover::{DiscoverError, DiscoverErrorKind, Discovered};
pub use go_modules::{GoModule, GoModules, GoWorkspace};
use zenith_lang::Resolved;

/// What a Go import path refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoResolved {
    /// A package in the project.
    Internal {
        /// The directory of the package, as a POSIX path relative to the
        /// project root, such as `services/api/store`. The root is `""`.
        dir: String,
    },

    /// A package of the standard library.
    Std {
        /// The import path, such as `net/http`.
        package: String,
    },

    /// A package of a module that is not in the project.
    External {
        /// The module path, such as `github.com/pkg/errors`.
        module: String,

        /// Whether the `go.mod` of the importing module requires it. When it
        /// does not, the go.mod is probably untidy, and the module is a guess
        /// from the shape of the import path.
        required: bool,
    },

    /// The pseudo-package `"C"`, which gives access to C code with cgo.
    Cgo,
}

impl From<GoResolved> for Resolved {
    /// A package of the project is imported as a whole, and the standard
    /// library and cgo are builtins of the `go:` scheme. A dependency is named
    /// by its module, and is found when the `go.mod` requires it.
    fn from(resolved: GoResolved) -> Self {
        match resolved {
            GoResolved::Internal { dir } => Resolved::Package(dir),
            GoResolved::Std { package } => Resolved::Builtin(format!("go:{package}")),
            GoResolved::External { module, required } => {
                Resolved::External { package: module, found: required }
            }
            GoResolved::Cgo => Resolved::Builtin("go:C".to_owned()),
        }
    }
}
