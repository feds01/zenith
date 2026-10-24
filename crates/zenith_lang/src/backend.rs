//! The interface through which the pipeline runs a language: finding its
//! packages, parsing its files and resolving their imports.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use zenith_utils::fs::FileSystem;

use crate::{ParsedFile, ResolveFailure, Resolved, Workspace};

/// What the pipeline asks of every backend it sets up.
#[derive(Debug, Clone, Copy)]
pub struct BackendSettings<'a> {
    /// The canonical project root.
    pub root: &'a Path,

    /// The file system that the project's files are read from, in which the
    /// root is canonical.
    pub file_system: &'a Arc<dyn FileSystem>,

    /// Whether parsing records what each file exports, which following
    /// imports through re-exports needs, and costs time otherwise.
    pub record_exports: bool,
}

/// A language, set up for one project by its options. The pipeline runs it
/// in this order: [`discover`](Self::discover) once, then a
/// [`parser`](Self::parser) per thread over its files, then one
/// [`resolver`](Self::resolver) for every import of its files.
pub trait Backend: Send + Sync {
    /// The names of the manifest files that the walk collects for it, such
    /// as `package.json`.
    fn manifests(&self) -> &'static [&'static str];

    /// The scheme of the node ids of its external packages, without the `:`,
    /// such as `npm`. Builtins name their own scheme.
    fn external_scheme(&self) -> &'static str;

    /// Find the packages of the project among `manifests`, the walked files
    /// named one of the [manifest names](Self::manifests), sorted. It may keep
    /// what resolving needs.
    fn discover(&mut self, manifests: &[PathBuf]) -> Workspace;

    /// A parser of its files, for one thread, so that it can reuse its
    /// buffers from file to file.
    fn parser(&self) -> Box<dyn SourceParser>;

    /// A resolver of the imports of its files, shared between threads. Run
    /// [`discover`](Self::discover) first.
    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure>;
}

/// Parses the source files of a language.
pub trait SourceParser {
    /// What the file at `path`, whose text is `source`, imports.
    fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile;
}

/// Resolves the imports of a language's files.
pub trait ImportResolver: Sync {
    /// What `specifier`, imported by the file at `path`, refers to.
    fn resolve(&self, path: &Path, specifier: &str) -> Resolved;
}
