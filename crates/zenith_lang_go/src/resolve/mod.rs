//! Turning a Go import path into what it refers to: a package of the project,
//! of the standard library, or of a dependency.

mod discover;
mod go_modules;
mod heuristic;

pub use discover::{DiscoverError, DiscoverErrorKind, Discovered};
pub use go_modules::{GoModule, GoModules, GoWorkspace};

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
