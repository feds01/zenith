//! The resolver, and how it classifies what a specifier resolves to.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use oxc_resolver::{
    AliasValue, ResolveError, ResolveOptions, TsconfigDiscovery, TsconfigOptions,
    TsconfigReferences,
};
use zenith_utils::path::relative_posix;

use crate::{
    settings::{ResolverSettings, TsconfigMode},
    specifier::{package_from_path, package_name, squash_builtin},
};

/// The files that an import with a JavaScript extension may stand for, in the
/// order TypeScript tries them, so `./util.js` finds `util.ts`.
const EXTENSION_ALIASES: [(&str, &[&str]); 4] = [
    (".js", &[".ts", ".tsx", ".d.ts", ".js", ".jsx"]),
    (".jsx", &[".tsx", ".ts", ".d.ts", ".jsx", ".js"]),
    (".mjs", &[".mts", ".d.mts", ".mjs"]),
    (".cjs", &[".cts", ".d.cts", ".cjs"]),
];

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
    },

    /// A runtime builtin without its subpath, e.g. `node:fs`. Node builtins
    /// always have the `node:` prefix.
    Builtin(String),

    /// The specifier could not be resolved.
    Unresolved(UnresolvedReason),
}

/// Why a specifier could not be resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum UnresolvedReason {
    /// The resolver could not resolve it.
    Failed(ResolveFailure),

    /// It resolved to a file outside the project root that is in no package.
    OutsideRoot(PathBuf),
}

impl fmt::Display for UnresolvedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(failure) => write!(f, "{failure}"),
            Self::OutsideRoot(path) => {
                write!(f, "resolved to `{}`, outside the project and in no package", path.display())
            }
        }
    }
}

/// An error from `oxc_resolver`, the resolver that zenith uses. Only its
/// message is part of this crate's API.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolveFailure(ResolveError);

impl fmt::Display for ResolveFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Why a [`Resolver`] can't be created from its settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolverError {
    /// The `tsconfig.json` to use for every file does not exist.
    TsconfigNotFound(PathBuf),

    /// An extension does not start with `.`.
    InvalidExtension(String),
}

impl fmt::Display for ResolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TsconfigNotFound(path) => {
                write!(f, "the tsconfig file `{}` does not exist", path.display())
            }
            Self::InvalidExtension(extension) => {
                write!(f, "the extension `{extension}` does not start with `.`")
            }
        }
    }
}

impl std::error::Error for ResolverError {}

/// Resolves the specifiers of one project. Share one between threads: its
/// caches are internally synchronised.
#[derive(Debug)]
pub struct Resolver {
    /// The configured `oxc_resolver`.
    inner: oxc_resolver::Resolver,

    /// The canonical project root.
    root: PathBuf,
}

impl Resolver {
    /// Create a resolver.
    pub fn new(settings: ResolverSettings) -> Result<Self, ResolverError> {
        let ResolverSettings { root, tsconfig, conditions, main_fields, extensions, aliases } =
            settings;

        if let Some(extension) = extensions.iter().find(|extension| !extension.starts_with('.')) {
            return Err(ResolverError::InvalidExtension(extension.clone()));
        }
        let tsconfig = match tsconfig {
            TsconfigMode::Auto => Some(TsconfigDiscovery::Auto),
            TsconfigMode::Off => None,
            TsconfigMode::File(path) if !path.is_file() => {
                return Err(ResolverError::TsconfigNotFound(path));
            }
            TsconfigMode::File(path) => Some(TsconfigDiscovery::Manual(TsconfigOptions {
                config_file: path,
                references: TsconfigReferences::Auto,
            })),
        };

        let strings = |items: &[&str]| items.iter().map(|item| item.to_string()).collect();
        let options = ResolveOptions {
            tsconfig,
            alias: aliases
                .into_iter()
                .map(|(name, path)| {
                    (name, vec![AliasValue::Path(path.to_string_lossy().into_owned())])
                })
                .collect(),
            condition_names: conditions,
            main_fields,
            extensions,
            extension_alias: EXTENSION_ALIASES
                .iter()
                .map(|(extension, aliases)| (extension.to_string(), strings(aliases)))
                .collect(),
            builtin_modules: true,
            // Resolve symlinks, so that linked workspace packages land on
            // their sources.
            symlinks: true,
            // Resolution must not depend on the environment.
            node_path: false,
            ..ResolveOptions::default()
        };

        Ok(Self { inner: oxc_resolver::Resolver::new(options), root })
    }

    /// Resolve `specifier`, as imported by the file at the absolute path
    /// `file`.
    pub fn resolve(&self, file: &Path, specifier: &str) -> Resolved {
        match self.inner.resolve_file(file, specifier) {
            Ok(resolution) => self.classify(specifier, resolution.path()),
            Err(ResolveError::Builtin { mut resolved, .. }) => {
                resolved.truncate(squash_builtin(&resolved).len());
                Resolved::Builtin(resolved)
            }
            Err(error) => failed(error),
        }
    }

    /// Classify the file that `specifier` resolved to.
    fn classify(&self, specifier: &str, path: &Path) -> Resolved {
        if let Ok(inside) = path.strip_prefix(&self.root)
            && !inside.components().any(|component| component.as_os_str() == "node_modules")
            && let Some(relative) = relative_posix(&self.root, path)
        {
            return Resolved::Internal(relative);
        }
        match package_name(specifier).map(str::to_string).or_else(|| package_from_path(path)) {
            Some(package) => Resolved::External { package },
            None => Resolved::Unresolved(UnresolvedReason::OutsideRoot(path.to_path_buf())),
        }
    }
}

/// An unresolved specifier, for a resolver error.
fn failed(error: ResolveError) -> Resolved {
    Resolved::Unresolved(UnresolvedReason::Failed(ResolveFailure(error)))
}

#[cfg(test)]
mod tests {
    use std::{fs, thread};

    use tempfile::TempDir;
    use zenith_utils::path::adjust_canonicalisation;

    use super::*;

    /// A project in a temporary directory.
    struct Project {
        /// Keeps the directory alive.
        _dir: TempDir,

        /// The canonical project root.
        root: PathBuf,
    }

    impl Project {
        /// A project with the given files, in a fresh temporary directory.
        fn new(files: &[(&str, &str)]) -> Self {
            Self::nested("", files)
        }

        /// A project rooted at `root` inside a fresh temporary directory, with
        /// the given files relative to the temporary directory. This puts
        /// files outside the root.
        fn nested(root: &str, files: &[(&str, &str)]) -> Self {
            let dir = TempDir::new().unwrap();
            let base = adjust_canonicalisation(dir.path());
            for (file, contents) in files {
                let path = base.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, contents).unwrap();
            }
            let root = base.join(root);
            fs::create_dir_all(&root).unwrap();
            Self { _dir: dir, root }
        }

        /// The absolute path of a file in the project.
        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        /// The default settings for the project.
        fn settings(&self) -> ResolverSettings {
            ResolverSettings::new(self.root.clone())
        }
    }

    fn internal(path: &str) -> Resolved {
        Resolved::Internal(path.to_string())
    }

    fn external(package: &str) -> Resolved {
        Resolved::External { package: package.to_string() }
    }

    fn builtin(name: &str) -> Resolved {
        Resolved::Builtin(name.to_string())
    }

    fn is_unresolved(resolved: &Resolved) -> bool {
        matches!(resolved, Resolved::Unresolved(UnresolvedReason::Failed(_)))
    }

    #[test]
    fn resolves_relative_imports_like_typescript() {
        let project = Project::new(&[
            ("web/src/main.ts", ""),
            ("web/src/lib/util.ts", ""),
            ("web/src/lib/index.ts", ""),
            ("web/src/view.tsx", ""),
            ("web/src/types.d.ts", ""),
            ("web/src/worker.mts", ""),
            ("web/src/legacy.cts", ""),
            ("web/src/plain.js", ""),
            ("web/src/logo.svg", ""),
            ("web/src/data.json", "{}"),
        ]);
        let resolver = Resolver::new(project.settings()).unwrap();
        let from = project.path("web/src/main.ts");
        let resolve = |specifier: &str| resolver.resolve(&from, specifier);

        assert_eq!(resolve("./lib/util"), internal("web/src/lib/util.ts"));
        assert_eq!(resolve("../src/lib/util"), internal("web/src/lib/util.ts"));
        assert_eq!(resolve("./lib"), internal("web/src/lib/index.ts"));
        assert_eq!(resolve("./view"), internal("web/src/view.tsx"));
        assert_eq!(resolve("./plain"), internal("web/src/plain.js"));
        assert_eq!(resolve("./data.json"), internal("web/src/data.json"));
        assert_eq!(resolve("./logo.svg?react"), internal("web/src/logo.svg"));

        // JavaScript extensions stand for their TypeScript sources.
        assert_eq!(resolve("./lib/util.js"), internal("web/src/lib/util.ts"));
        assert_eq!(resolve("./view.js"), internal("web/src/view.tsx"));
        assert_eq!(resolve("./view.jsx"), internal("web/src/view.tsx"));
        assert_eq!(resolve("./types.js"), internal("web/src/types.d.ts"));
        assert_eq!(resolve("./worker.mjs"), internal("web/src/worker.mts"));
        assert_eq!(resolve("./legacy.cjs"), internal("web/src/legacy.cts"));
        assert_eq!(resolve("./plain.js"), internal("web/src/plain.js"));

        assert!(matches!(
            resolve("./missing"),
            Resolved::Unresolved(UnresolvedReason::Failed(ResolveFailure(ResolveError::NotFound(
                _
            ))))
        ));
        assert!(is_unresolved(&resolve("./missing.js")));
    }

    #[test]
    fn follows_the_nearest_tsconfig_paths() {
        let project = Project::new(&[
            (
                "tsconfig.base.json",
                r#"{ "compilerOptions": { "paths": { "@/*": ["./web/src/*"] } } }"#,
            ),
            ("web/tsconfig.json", r#"{ "extends": "../tsconfig.base.json" }"#),
            ("web/src/main.ts", ""),
            ("web/src/lib/format.ts", ""),
            (
                "admin/tsconfig.json",
                r#"{ "compilerOptions": { "paths": { "@/*": ["./app/*"], "~config": ["./config.ts"] } } }"#,
            ),
            ("admin/config.ts", ""),
            ("admin/app/main.ts", ""),
            ("admin/app/lib/format.ts", ""),
        ]);
        let resolver = Resolver::new(project.settings()).unwrap();
        let web = project.path("web/src/main.ts");
        let admin = project.path("admin/app/main.ts");

        assert_eq!(resolver.resolve(&web, "@/lib/format"), internal("web/src/lib/format.ts"));
        assert_eq!(resolver.resolve(&admin, "@/lib/format"), internal("admin/app/lib/format.ts"));
        assert_eq!(resolver.resolve(&admin, "~config"), internal("admin/config.ts"));
    }

    #[test]
    fn tsconfig_can_be_off_or_one_file() {
        let project = Project::new(&[
            ("tsconfig.json", r#"{ "compilerOptions": { "paths": { "@/*": ["./src/*"] } } }"#),
            (
                "tsconfig.other.json",
                r#"{ "compilerOptions": { "paths": { "@/*": ["./other/*"] } } }"#,
            ),
            ("src/main.ts", ""),
            ("src/util.ts", ""),
            ("other/util.ts", ""),
        ]);
        let from = project.path("src/main.ts");

        let mut settings = project.settings();
        settings.tsconfig = TsconfigMode::Off;
        let resolver = Resolver::new(settings).unwrap();
        assert!(is_unresolved(&resolver.resolve(&from, "@/util")));

        let mut settings = project.settings();
        settings.tsconfig = TsconfigMode::File(project.path("tsconfig.other.json"));
        let resolver = Resolver::new(settings).unwrap();
        assert_eq!(resolver.resolve(&from, "@/util"), internal("other/util.ts"));
    }

    #[test]
    fn invalid_settings_are_errors() {
        let project = Project::new(&[]);

        let mut settings = project.settings();
        let missing = project.path("tsconfig.missing.json");
        settings.tsconfig = TsconfigMode::File(missing.clone());
        let error = Resolver::new(settings).unwrap_err();
        assert_eq!(error, ResolverError::TsconfigNotFound(missing.clone()));
        assert_eq!(
            error.to_string(),
            format!("the tsconfig file `{}` does not exist", missing.display())
        );

        let mut settings = project.settings();
        settings.extensions.push("ts".to_string());
        let error = Resolver::new(settings).unwrap_err();
        assert_eq!(error, ResolverError::InvalidExtension("ts".to_string()));
        assert_eq!(error.to_string(), "the extension `ts` does not start with `.`");
    }

    #[test]
    fn classifies_installed_packages_as_external() {
        let project = Project::new(&[
            ("web/src/main.ts", ""),
            (
                "node_modules/@vendor/sdk/package.json",
                r#"{ "name": "@vendor/sdk", "main": "index.js" }"#,
            ),
            ("node_modules/@vendor/sdk/index.js", ""),
            ("node_modules/@vendor/sdk/client.js", ""),
            ("node_modules/@vendor/sdk/styles.css", ""),
            (
                "node_modules/chart-kit/package.json",
                r#"{ "name": "chart-kit", "exports": { ".": "./index.js" } }"#,
            ),
            ("node_modules/chart-kit/index.js", ""),
            ("node_modules/chart-kit/internal.js", ""),
        ]);
        let resolver = Resolver::new(project.settings()).unwrap();
        let from = project.path("web/src/main.ts");
        let resolve = |specifier: &str| resolver.resolve(&from, specifier);

        assert_eq!(resolve("@vendor/sdk"), external("@vendor/sdk"));
        assert_eq!(resolve("@vendor/sdk/client"), external("@vendor/sdk"));
        assert_eq!(resolve("chart-kit"), external("chart-kit"));
        // A relative import into `node_modules` is named from the path.
        assert_eq!(resolve("../../node_modules/@vendor/sdk/styles.css"), external("@vendor/sdk"));
    }

    #[test]
    fn classifies_files_outside_the_root() {
        let project = Project::nested(
            "repo",
            &[
                ("repo/web/main.ts", ""),
                ("shared/util.ts", ""),
                (
                    "node_modules/@vendor/sdk/package.json",
                    r#"{ "name": "@vendor/sdk", "main": "index.js" }"#,
                ),
                ("node_modules/@vendor/sdk/index.js", ""),
            ],
        );
        let resolver = Resolver::new(project.settings()).unwrap();
        let from = project.path("web/main.ts");

        assert_eq!(resolver.resolve(&from, "@vendor/sdk"), external("@vendor/sdk"));
        let outside = resolver.resolve(&from, "../../shared/util");
        let shared = project.root.parent().unwrap().join("shared").join("util.ts");
        assert_eq!(outside, Resolved::Unresolved(UnresolvedReason::OutsideRoot(shared)));
    }

    #[test]
    fn classifies_builtins() {
        let project = Project::new(&[("main.ts", "")]);
        let resolver = Resolver::new(project.settings()).unwrap();
        let from = project.path("main.ts");
        let resolve = |specifier: &str| resolver.resolve(&from, specifier);

        assert_eq!(resolve("fs"), builtin("node:fs"));
        assert_eq!(resolve("fs/promises"), builtin("node:fs"));
        assert_eq!(resolve("node:fs"), builtin("node:fs"));
        assert_eq!(resolve("node:fs/promises"), builtin("node:fs"));
        assert_eq!(resolve("node:test"), builtin("node:test"));
    }

    #[test]
    fn resolves_aliases() {
        let project = Project::new(&[
            ("web/src/main.ts", ""),
            ("web/src/types/user.ts", ""),
            ("web/src/types/index.ts", ""),
        ]);
        let mut settings = project.settings();
        settings.aliases = vec![("types".to_string(), project.path("web/src/types"))];
        let resolver = Resolver::new(settings).unwrap();
        let from = project.path("web/src/main.ts");

        assert_eq!(resolver.resolve(&from, "types"), internal("web/src/types/index.ts"));
        assert_eq!(resolver.resolve(&from, "types/user"), internal("web/src/types/user.ts"));
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlinked_workspace_packages() {
        let project = Project::new(&[
            ("web/main.ts", ""),
            ("packages/ui/package.json", r#"{ "name": "@acme/ui", "exports": "./src/index.ts" }"#),
            ("packages/ui/src/index.ts", ""),
        ]);
        fs::create_dir_all(project.path("node_modules/@acme")).unwrap();
        std::os::unix::fs::symlink("../../packages/ui", project.path("node_modules/@acme/ui"))
            .unwrap();
        let resolver = Resolver::new(project.settings()).unwrap();
        assert_eq!(
            resolver.resolve(&project.path("web/main.ts"), "@acme/ui"),
            internal("packages/ui/src/index.ts")
        );
    }

    #[test]
    fn resolvers_are_shared_between_threads() {
        let project = Project::new(&[("web/main.ts", ""), ("web/util.ts", "")]);
        let resolver = Resolver::new(project.settings()).unwrap();
        let from = project.path("web/main.ts");
        thread::scope(|scope| {
            let handles: Vec<_> =
                (0..4).map(|_| scope.spawn(|| resolver.resolve(&from, "./util"))).collect();
            for handle in handles {
                assert_eq!(handle.join().unwrap(), internal("web/util.ts"));
            }
        });
    }
}
