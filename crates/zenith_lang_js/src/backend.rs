//! JavaScript and TypeScript as the pipeline runs them: workspace packages
//! from `package.json` files, parsing with oxc, and resolving with
//! `oxc_resolver`.

use std::path::{Path, PathBuf};

use zenith_lang::{
    Backend, ImportResolver, ParsedFile, ResolveFailure, Resolved, SourceParser, Workspace,
};

use crate::{JsConfig, ParseOptions, Parser, Resolver, discover_packages};

/// JavaScript and TypeScript, set up for one project.
pub(crate) struct JsBackend {
    /// The options of `[languages.js]`.
    pub(crate) config: JsConfig,

    /// The canonical project root.
    pub(crate) root: PathBuf,

    /// What the parsers record.
    pub(crate) parse: ParseOptions,

    /// The workspace packages, once they are discovered, which resolve
    /// without `node_modules`.
    pub(crate) workspace: Workspace,
}

impl Backend for JsBackend {
    fn manifests(&self) -> &'static [&'static str] {
        &["package.json"]
    }

    fn external_scheme(&self) -> &'static str {
        "npm"
    }

    fn discover(&mut self, manifests: &[PathBuf]) -> Workspace {
        self.workspace = discover_packages(&self.root, manifests);
        self.workspace.clone()
    }

    fn parser(&self) -> Box<dyn SourceParser> {
        Box::new(Parser::with_options(self.parse))
    }

    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure> {
        let settings = self.config.resolver_settings(&self.root, &self.workspace);
        let resolver =
            Resolver::new(settings).map_err(|error| ResolveFailure::new(error.to_string()))?;
        Ok(Box::new(resolver))
    }
}

impl SourceParser for Parser {
    fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile {
        Parser::parse_source(self, path, source)
    }
}

impl ImportResolver for Resolver {
    fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        Resolver::resolve(self, path, specifier)
    }
}
