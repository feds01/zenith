//! Haskell as the pipeline runs it: packages from their `.cabal`
//! descriptions, the libraries that their builds installed, parsing with
//! zenith's own parser, and resolving as GHC would.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use zenith_graph::ImportKind;
use zenith_lang::{
    Backend, ImportResolver, ParsedFile, ResolveFailure, Resolved, SourceParser, Workspace,
};
use zenith_utils::{fs::FileSystem, glob::NamePattern};

use crate::{
    packages::Packages,
    parse::{Dialect, Parser},
    resolve::Resolver,
};

/// Haskell, set up for one project.
pub(crate) struct HaskellBackend {
    /// The canonical project root.
    root: PathBuf,

    /// The file system that the project's files are read from.
    file_system: Arc<dyn FileSystem>,

    /// The packages of the project and what its builds installed, once they
    /// are discovered.
    packages: Packages,

    /// Patterns of the specifiers that are expected not to resolve.
    ignore_unresolved: Vec<NamePattern>,
}

impl HaskellBackend {
    /// The backend of the project at `root` in `file_system`, which must be
    /// canonical there, which expects the specifiers that match
    /// `ignore_unresolved` not to resolve.
    pub(crate) fn new(
        root: &Path,
        file_system: Arc<dyn FileSystem>,
        ignore_unresolved: Vec<NamePattern>,
    ) -> Self {
        let packages = Packages::default();
        Self { root: root.to_path_buf(), file_system, packages, ignore_unresolved }
    }
}

impl Backend for HaskellBackend {
    fn manifests(&self) -> &'static [&'static str] {
        &["*.cabal", "cabal.project", "stack.yaml"]
    }

    fn external_scheme(&self) -> &'static str {
        "hackage"
    }

    fn discover(&mut self, manifests: &[PathBuf]) -> Workspace {
        self.packages = Packages::discover(&*self.file_system, &self.root, manifests);
        self.packages.workspace(&self.root)
    }

    fn parser(&self) -> Box<dyn SourceParser> {
        Box::new(Parser::new())
    }

    /// Mark the imports of each file that only test suites and benchmarks
    /// compile as test imports, which its package alone tells.
    fn link(&mut self, files: &mut dyn Iterator<Item = (PathBuf, &mut ParsedFile)>) {
        for (path, parsed) in files {
            if self
                .packages
                .containing(&path)
                .is_some_and(|package| package.compiles_only_tests(&path))
            {
                parsed.imports.iter_mut().for_each(|import| import.kind = ImportKind::Test);
            }
        }
    }

    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure> {
        let file_system = &*self.file_system;
        Ok(Box::new(Resolver::new(
            &self.root,
            file_system,
            &self.packages,
            &self.ignore_unresolved,
        )))
    }

    // What an import resolves to also depends on what `cabal`, `stack` and `ghc`
    // say of the build, which is not read through the file system, so it is not
    // kept between runs.
}

impl SourceParser for Parser {
    fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile {
        self.parse(source, Dialect::of(path)).into_parsed(source)
    }
}

impl ImportResolver for Resolver<'_> {
    fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        Resolver::resolve(self, path, specifier)
    }
}
