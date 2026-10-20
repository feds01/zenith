//! What an import specifier resolves to, as every language reports it.

use std::{fmt, path::PathBuf};

/// What a specifier resolves to.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// A file inside the project, as a POSIX path relative to the root, e.g.
    /// `web/src/app.ts`.
    Internal(String),

    /// An external package: one under `node_modules` or outside the root.
    External {
        /// The package name, without any subpath, e.g. `@tanstack/react-query`.
        package: String,

        /// Whether the specifier resolved to a file. When it didn't (because
        /// `node_modules` is not installed, say), the package is named from
        /// the specifier alone.
        found: bool,
    },

    /// A runtime builtin without its subpath, e.g. `node:fs` or `bun:sqlite`.
    /// Node builtins always have the `node:` prefix.
    Builtin(String),

    /// The specifier could not be resolved.
    Unresolved(Box<UnresolvedReason>),

    /// The specifier could not be resolved, and it matches one of the
    /// `ignore_unresolved` patterns, so that is expected.
    Ignored(Box<UnresolvedReason>),
}

impl Resolved {
    /// An unresolved specifier, for this reason.
    pub fn unresolved(reason: UnresolvedReason) -> Self {
        Self::Unresolved(Box::new(reason))
    }
}

// A `Resolved` is made for every import, so it is kept small: the reason of an
// unresolved import, the rare case, is boxed.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Resolved>() == 32);

/// Why a specifier could not be resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum UnresolvedReason {
    /// The language's resolver could not resolve it.
    Failed(ResolveFailure),

    /// A workspace package's `exports` map doesn't export the subpath.
    NotExported {
        /// The package name.
        package: String,

        /// The subpath, as an `exports` key, e.g. `./src/internal`.
        subpath: String,
    },

    /// It resolved to a file outside the project root that is in no package.
    OutsideRoot(PathBuf),

    /// It has a URL scheme other than `node:` and `bun:`, such as `virtual:`.
    UnsupportedScheme(String),
}

impl UnresolvedReason {
    /// Whether the importing file's project configuration (a `tsconfig.json`,
    /// say) couldn't be loaded, so what the specifier may have needed from it
    /// (its `paths`, say) was missing.
    pub fn is_config_error(&self) -> bool {
        matches!(self, Self::Failed(failure) if failure.is_config_error())
    }
}

impl fmt::Display for UnresolvedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(failure) => write!(f, "{failure}"),
            Self::NotExported { package, subpath } => {
                write!(f, "`{subpath}` is not exported by `{package}`")
            }
            Self::OutsideRoot(path) => {
                write!(f, "resolved to `{}`, outside the project and in no package", path.display())
            }
            Self::UnsupportedScheme(scheme) => write!(f, "unsupported `{scheme}:` specifier"),
        }
    }
}

/// An error from a language's resolver. Only its message is part of the API,
/// so it doesn't depend on the resolver that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveFailure {
    /// The resolver's message.
    message: String,

    /// Whether the resolver failed to load the project configuration.
    config_error: bool,
}

impl ResolveFailure {
    /// A failure with this message.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), config_error: false }
    }

    /// A failure to load the project configuration, such as a
    /// `tsconfig.json`, with this message.
    pub fn config_error(message: impl Into<String>) -> Self {
        Self { message: message.into(), config_error: true }
    }

    /// Whether the resolver failed to load the project configuration.
    pub fn is_config_error(&self) -> bool {
        self.config_error
    }
}

impl fmt::Display for ResolveFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
