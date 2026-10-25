//! Rust as the pipeline runs it: crates from `Cargo.toml` files, parsing with
//! rust-analyzer's parser in each crate's edition, and resolving along the
//! module tree.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use zenith_graph::{ImportDetail, ImportedNames};
use zenith_lang::{
    Backend, ImportResolver, ParsedFile, ResolveFailure, Resolved, SourceParser, Workspace,
};
use zenith_utils::{fs::FileSystem, glob::NamePattern};

use crate::{
    crates::{Crates, Editions},
    modules::{Items, TestModules},
    parse::Parser,
    resolve::Resolver,
};

/// Rust, set up for one project.
pub(crate) struct RustBackend {
    /// The canonical project root.
    root: PathBuf,

    /// The file system that the project's files are read from.
    file_system: Arc<dyn FileSystem>,

    /// The crates of the project, once they are discovered.
    crates: Crates,

    /// The edition of each crate, which the parsers share.
    editions: Arc<Editions>,

    /// Patterns of the paths that are expected not to resolve.
    ignore_unresolved: Vec<NamePattern>,

    /// The modules that each file declares for its tests, kept from one link
    /// to the next.
    test_modules: TestModules,

    /// The items of each file, kept from one link to the next.
    items: Items,
}

impl RustBackend {
    /// The backend of the project at `root` in `file_system`, which must be
    /// canonical there, which expects the paths that match
    /// `ignore_unresolved` not to resolve.
    pub(crate) fn new(
        root: &Path,
        file_system: Arc<dyn FileSystem>,
        ignore_unresolved: Vec<NamePattern>,
    ) -> Self {
        let (crates, editions) = (Crates::default(), Arc::default());
        let (test_modules, items) = (TestModules::default(), Items::default());
        let root = root.to_path_buf();
        Self { root, file_system, crates, editions, ignore_unresolved, test_modules, items }
    }
}

impl Backend for RustBackend {
    fn manifests(&self) -> &'static [&'static str] {
        &["Cargo.toml"]
    }

    fn external_scheme(&self) -> &'static str {
        "crate"
    }

    fn discover(&mut self, manifests: &[PathBuf]) -> Workspace {
        self.crates = Crates::discover(&*self.file_system, &self.root, manifests);
        self.editions = Arc::new(self.crates.editions());
        self.crates.workspace(&self.root)
    }

    fn parser(&self) -> Box<dyn SourceParser> {
        Box::new(Parser::new(Arc::clone(&self.editions)))
    }

    fn link(&mut self, files: &mut dyn Iterator<Item = (PathBuf, &mut ParsedFile)>) {
        let mut files: Vec<_> = files.collect();
        self.crates.mark_test_code(&mut files, &mut self.test_modules);
        self.items.update(&files);
    }

    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure> {
        let (root, file_system, crates) = (&self.root, &*self.file_system, &self.crates);
        Ok(Box::new(Resolver::new(root, file_system, crates, &self.items, &self.ignore_unresolved)))
    }
}

impl SourceParser for Parser {
    fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile {
        Parser::parse_source(self, path, source)
    }
}

impl ImportResolver for Resolver<'_> {
    fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        Resolver::resolve(self, path, specifier, None)
    }

    fn resolve_import(&self, path: &Path, import: &ImportDetail) -> Resolved {
        let name = match &import.names {
            ImportedNames::Names(names) => names.first().map(|name| name.name.as_str()),
            ImportedNames::All => None,
        };
        Resolver::resolve(self, path, &import.specifier, name)
    }
}
