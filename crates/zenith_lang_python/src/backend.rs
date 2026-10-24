//! Python as the pipeline runs it: packages from `pyproject.toml` files,
//! parsing with Ruff's parser, and resolving against the roots of the project.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use zenith_lang::{
    Backend, ImportResolver, ParsedFile, ResolveFailure, Resolved, SourceParser, Workspace,
};
use zenith_utils::{fs::FileSystem, glob::NamePattern};

use crate::{modules::Roots, packages::Package, parse::Parser, resolve::Resolver};

/// Python, set up for one project.
pub(crate) struct PythonBackend {
    /// The canonical project root.
    root: PathBuf,

    /// The file system that the project's files are read from.
    file_system: Arc<dyn FileSystem>,

    /// The roots that the configuration names, relative to the project root,
    /// or `None` to find them from the packages.
    configured: Option<Vec<String>>,

    /// The roots that absolute imports start from, once they are discovered.
    roots: Roots,

    /// Patterns of the module paths that are expected not to resolve.
    ignore_unresolved: Vec<NamePattern>,
}

impl PythonBackend {
    /// The backend of the project at `root` in `file_system`, which must be
    /// canonical there, with the roots that the configuration names, if it
    /// does, which expects the module paths that match `ignore_unresolved`
    /// not to resolve.
    pub(crate) fn new(
        root: &Path,
        file_system: Arc<dyn FileSystem>,
        configured: Option<Vec<String>>,
        ignore_unresolved: Vec<NamePattern>,
    ) -> Self {
        let roots = Roots::default();
        Self { root: root.to_path_buf(), file_system, configured, roots, ignore_unresolved }
    }
}

impl Backend for PythonBackend {
    fn manifests(&self) -> &'static [&'static str] {
        &["pyproject.toml"]
    }

    fn external_scheme(&self) -> &'static str {
        "pypi"
    }

    fn discover(&mut self, manifests: &[PathBuf]) -> Workspace {
        let file_system = &*self.file_system;
        let packages = Package::read_all(file_system, manifests);
        self.roots = match &self.configured {
            Some(names) => Roots::configured(file_system, &self.root, names),
            None => Roots::inferred(file_system, &self.root, &packages),
        };
        Package::workspace(&self.root, &packages)
    }

    fn parser(&self) -> Box<dyn SourceParser> {
        Box::new(Parser::new())
    }

    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure> {
        let file_system = self.file_system.clone();
        Ok(Box::new(Resolver::new(&self.root, file_system, &self.roots, &self.ignore_unresolved)))
    }
}

impl SourceParser for Parser {
    fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile {
        Parser::parse_source(self, path, source)
    }
}

impl ImportResolver for Resolver<'_> {
    fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        Resolver::resolve(self, path, specifier)
    }
}
