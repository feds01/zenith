//! The interface through which the pipeline runs a language: finding its
//! packages, parsing its files and resolving their imports.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use zenith_graph::ImportDetail;
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
/// [`parser`](Self::parser) per thread over its files, then
/// [`link`](Self::link) once over all of them, then one
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

    /// Settle what only all of its parsed `files` tell, each with its path,
    /// such as which of them only tests compile, as a linker joins the files
    /// of a program. What a file parsed to depends on it alone, so that it can
    /// be cached; what this changes is not cached. The files are those that
    /// were just parsed: every file of its language, or, when a session is
    /// updated, the changed ones alone, so it keeps what it learns of each
    /// file for the next time. They are made as they are taken, so a
    /// language that takes none costs nothing, as by default.
    fn link(&mut self, files: &mut dyn Iterator<Item = (PathBuf, &mut ParsedFile)>) {
        let _ = files;
    }

    /// A resolver of the imports of its files, shared between threads. Run
    /// [`discover`](Self::discover) first.
    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure>;

    /// Whether what an import resolves to depends only on its specifier, the
    /// path of its file, the manifests that [`discover`](Self::discover) was
    /// given, and what the backend reads through its file system as it finds
    /// the packages and resolves, so that it can be kept between runs while
    /// what was read is unchanged. A language whose imports also resolve by
    /// the names that they take, as an override of
    /// [`ImportResolver::resolve_import`] may, or by what other files hold,
    /// doesn't.
    fn resolves_by_specifier(&self) -> bool {
        false
    }
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

    /// What `import`, made by the file at `path`, refers to: what its
    /// specifier does, unless the names that it takes tell otherwise, as in a
    /// language whose items and modules may share a name.
    fn resolve_import(&self, path: &Path, import: &ImportDetail) -> Resolved {
        self.resolve(path, &import.specifier)
    }
}
