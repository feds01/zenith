//! Go as the pipeline runs it: modules from `go.mod` and `go.work` files,
//! parsing with tree-sitter, and resolving against the modules.

use std::path::{Path, PathBuf};

use log::warn;
use zenith_lang::{
    Backend, ImportResolver, ParsedFile, ResolveFailure, Resolved, SourceParser, Workspace,
};
use zenith_utils::{fs::read_text, glob::NamePattern, path::relative_posix};

use crate::{parse::Parser, resolve::GoModules};

/// Go, set up for one project.
pub(crate) struct GoBackend {
    /// The canonical project root.
    root: PathBuf,

    /// The modules of the project, once they are discovered.
    modules: GoModules,

    /// Patterns of the import paths that are expected not to resolve.
    ignore_unresolved: Vec<NamePattern>,
}

impl GoBackend {
    /// The backend of the project at `root`, which must be canonical, which
    /// expects the import paths that match `ignore_unresolved` not to
    /// resolve. It has no modules until it discovers them.
    pub(crate) fn new(root: &Path, ignore_unresolved: Vec<NamePattern>) -> Self {
        Self { root: root.to_path_buf(), modules: GoModules::default(), ignore_unresolved }
    }
}

impl Backend for GoBackend {
    fn manifests(&self) -> &'static [&'static str] {
        &["go.mod", "go.work"]
    }

    fn external_scheme(&self) -> &'static str {
        "gomod"
    }

    fn discover(&mut self, manifests: &[PathBuf]) -> Workspace {
        let files = manifests.iter().filter_map(|manifest| relative_posix(&self.root, manifest));
        let found = GoModules::discover(files, |file| read_text(&self.root.join(file)));
        for error in &found.errors {
            warn!("{error}");
        }
        self.modules = found.modules;
        self.modules.workspace(&self.root)
    }

    fn parser(&self) -> Box<dyn SourceParser> {
        Box::new(Parser::new())
    }

    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure> {
        Ok(Box::new(GoResolver {
            root: &self.root,
            modules: &self.modules,
            ignore_unresolved: &self.ignore_unresolved,
        }))
    }
}

impl SourceParser for Parser {
    fn parse_source(&mut self, _path: &Path, source: &str) -> ParsedFile {
        self.parse(source).into_parsed(source)
    }
}

/// Resolves the imports of Go files against the modules of the project.
struct GoResolver<'a> {
    /// The canonical project root.
    root: &'a Path,

    /// The modules of the project.
    modules: &'a GoModules,

    /// Patterns of the import paths that are expected not to resolve.
    ignore_unresolved: &'a [NamePattern],
}

impl ImportResolver for GoResolver<'_> {
    fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        // Imports resolve from the directory of the importing file.
        let dir = path.parent().and_then(|dir| relative_posix(self.root, dir)).unwrap_or_default();
        let resolved: Resolved = self.modules.resolve(&dir, specifier).into();
        resolved.ignoring_unresolved(specifier, self.ignore_unresolved)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_config::LanguageOptions;
    use zenith_lang::BackendSettings;

    use super::*;
    use crate::GoConfig;

    /// A project with an `api` module and a root module, and the backend that
    /// discovered them.
    fn project() -> (TempDir, Box<dyn Backend>, Workspace) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("api")).unwrap();
        fs::write(root.join("go.mod"), "module example.com/app\n").unwrap();
        fs::write(
            root.join("api/go.mod"),
            "module example.com/api\nrequire github.com/pkg/errors v0.9.1\n",
        )
        .unwrap();
        let settings = BackendSettings { root: &root, record_exports: false };
        let mut backend = GoConfig::default().backend(settings);
        let manifests = [root.join("api/go.mod"), root.join("go.mod")];
        let workspace = backend.discover(&manifests);
        (dir, backend, workspace)
    }

    #[test]
    fn go_names_its_manifests_and_its_external_scheme() {
        let (_dir, backend, _) = project();
        assert_eq!(backend.manifests(), ["go.mod", "go.work"]);
        assert_eq!(backend.external_scheme(), "gomod");
    }

    #[test]
    fn the_modules_are_the_packages_of_the_workspace() {
        let (dir, _backend, workspace) = project();
        let root = dir.path().canonicalize().unwrap();
        let [api] = &workspace.packages[..] else {
            panic!("expected one package: {workspace:?}");
        };
        assert_eq!((api.name.as_str(), api.dir.as_str()), ("example.com/api", "api"));
        assert_eq!(api.path, root.join("api"));
        let app = workspace.root_package.expect("the root module");
        assert_eq!((app.name.as_str(), app.dir.as_str()), ("example.com/app", ""));
        assert_eq!(app.path, root);
    }

    #[test]
    fn a_project_without_modules_has_no_packages() {
        let dir = TempDir::new().unwrap();
        let settings = BackendSettings { root: dir.path(), record_exports: false };
        let workspace = GoConfig::default().backend(settings).discover(&[]);
        assert!(workspace.packages.is_empty() && workspace.root_package.is_none());
    }

    #[test]
    fn imports_resolve_from_the_directory_of_the_file() {
        let (dir, backend, _) = project();
        let root = dir.path().canonicalize().unwrap();
        let resolver = backend.resolver().unwrap();
        let resolve = |file: &str, specifier: &str| resolver.resolve(&root.join(file), specifier);

        assert_eq!(resolve("api/main.go", "example.com/api/store"), package("api/store"));
        // The nearest module is the importing one, which does not see the root's.
        assert_eq!(resolve("main.go", "example.com/app/util"), package("util"));
        assert_eq!(resolve("api/main.go", "net/http"), builtin("go:net/http"));
        assert_eq!(resolve("api/main.go", "C"), builtin("go:C"));
        assert_eq!(
            resolve("api/main.go", "github.com/pkg/errors"),
            Resolved::External { package: "github.com/pkg/errors".into(), found: true }
        );
        assert_eq!(
            resolve("api/main.go", "github.com/other/lib/sub"),
            Resolved::External { package: "github.com/other/lib".into(), found: false }
        );
    }

    fn package(dir: &str) -> Resolved {
        Resolved::Package(dir.to_string())
    }

    fn builtin(name: &str) -> Resolved {
        Resolved::Builtin(name.to_string())
    }
}
