//! Rust as the pipeline runs it: crates from `Cargo.toml` files, parsing with
//! tree-sitter, and resolving along the module tree.

use std::path::{Path, PathBuf};

use zenith_lang::{
    Backend, ImportResolver, ParsedFile, ResolveFailure, Resolved, SourceParser, Workspace,
};

use crate::{
    crates::{Crates, Discovery},
    parse::Parser,
    resolve::Resolver,
};

/// Rust, set up for one project.
pub(crate) struct RustBackend {
    /// The canonical project root.
    root: PathBuf,

    /// How the crates of the project are found.
    discovery: Discovery,

    /// The crates of the project, once they are discovered.
    crates: Crates,
}

impl RustBackend {
    /// The backend of the project at `root`, which must be canonical, which
    /// finds its crates the way that `discovery` says.
    pub(crate) fn new(root: &Path, discovery: Discovery) -> Self {
        Self { root: root.to_path_buf(), discovery, crates: Crates::default() }
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
        self.crates = Crates::discover(&self.root, manifests, self.discovery);
        self.crates.workspace(&self.root)
    }

    fn parser(&self) -> Box<dyn SourceParser> {
        Box::new(Parser::new())
    }

    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure> {
        Ok(Box::new(Resolver::new(&self.root, &self.crates)))
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
