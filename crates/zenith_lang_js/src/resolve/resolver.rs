//! The resolver, and how it classifies what a specifier resolves to.

use std::{
    borrow::Cow,
    fmt,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use oxc_resolver::{
    AliasValue, Resolution, ResolveError, ResolveOptions, ResolverGeneric, TsconfigDiscovery,
    TsconfigOptions, TsconfigReferences,
};
use rustc_hash::FxHashMap;
use serde_json::Value;
use zenith_lang::{ResolveFailure, Resolved, UnresolvedReason};
use zenith_utils::{
    fs::read_json,
    glob::NamePattern,
    path::{is_extension, relative_posix},
    specifier::{package_from_path, package_name, split_query, split_scheme, squash_builtin},
};

use super::{
    exports,
    links::LinkedFileSystem,
    settings::{ResolverSettings, TsconfigMode},
};

/// The files that an import with a JavaScript extension may stand for, in the
/// order TypeScript tries them, so `./util.js` finds `util.ts`.
const EXTENSION_ALIASES: [(&str, &[&str]); 4] = [
    (".js", &[".ts", ".tsx", ".d.ts", ".js", ".jsx"]),
    (".jsx", &[".tsx", ".ts", ".d.ts", ".jsx", ".js"]),
    (".mjs", &[".mts", ".d.mts", ".mjs"]),
    (".cjs", &[".cts", ".d.cts", ".cjs"]),
];

/// Classify an error from `oxc_resolver` as the reason a specifier is
/// unresolved.
fn failed(error: ResolveError) -> Resolved {
    Resolved::unresolved(UnresolvedReason::Failed(failure(&error)))
}

/// The error from `oxc_resolver`, without its type: only its message is part of
/// the shared API.
fn failure(error: &ResolveError) -> ResolveFailure {
    if is_tsconfig_error(error) {
        ResolveFailure::config_error(error.to_string())
    } else {
        ResolveFailure::new(error.to_string())
    }
}

/// Whether the resolver failed to load a `tsconfig.json`.
fn is_tsconfig_error(error: &ResolveError) -> bool {
    matches!(
        error,
        ResolveError::TsconfigNotFound(_)
            | ResolveError::TsconfigLoadFailed { .. }
            | ResolveError::TsconfigCircularExtend(_)
            | ResolveError::TsconfigSelfReference(_)
    )
}

/// Why a [`Resolver`] can't be created from its settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolverError {
    /// The `tsconfig.json` to use for every file does not exist.
    TsconfigNotFound(PathBuf),

    /// An extension is not a `.` followed by a name, such as `.ts`.
    InvalidExtension(String),
}

impl fmt::Display for ResolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TsconfigNotFound(path) => {
                write!(f, "the tsconfig file `{}` does not exist", path.display())
            }
            Self::InvalidExtension(extension) => {
                write!(f, "`{extension}` is not an extension, such as `.ts`")
            }
        }
    }
}

impl std::error::Error for ResolverError {}

/// Resolves the specifiers of one project. Share one between threads: its
/// caches are internally synchronised.
#[derive(Debug)]
pub struct Resolver {
    /// The configured `oxc_resolver`, which sees the workspace packages
    /// linked into the root `node_modules`.
    inner: ResolverGeneric<LinkedFileSystem>,

    /// The same, sharing its caches, but without `tsconfig.json` files, for
    /// files whose `tsconfig.json` can't be loaded.
    without_tsconfig: ResolverGeneric<LinkedFileSystem>,

    /// The files whose `tsconfig.json` couldn't be loaded, and why.
    tsconfig_errors: Mutex<FxHashMap<PathBuf, ResolveError>>,

    /// The canonical project root.
    root: PathBuf,

    /// The condition names, for workspace `exports` maps.
    conditions: Vec<String>,

    /// The workspace packages, by name.
    workspace: FxHashMap<String, WorkspaceEntry>,

    /// Patterns of specifiers that are expected not to resolve.
    ignore_unresolved: Vec<NamePattern>,
}

/// A workspace package, for resolving its imports that the links in
/// `node_modules` don't, and for saying which subpaths it doesn't export.
#[derive(Debug)]
struct WorkspaceEntry {
    /// Its `package.json`, which its subpaths are resolved from.
    manifest: PathBuf,

    /// Its `exports` map, if it has one.
    exports: Option<Value>,
}

impl Resolver {
    /// Create a resolver. The `package.json` of every workspace package is
    /// read now; one that can't be read counts as having no `exports` map.
    pub fn new(settings: ResolverSettings) -> Result<Self, ResolverError> {
        let ResolverSettings {
            root,
            tsconfig,
            conditions,
            main_fields,
            extensions,
            aliases,
            workspace_packages,
            ignore_unresolved,
        } = settings;

        if let Some(extension) = extensions.iter().find(|extension| !is_extension(extension)) {
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
            condition_names: conditions.clone(),
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

        let mut packages = FxHashMap::default();
        for (name, dir) in workspace_packages {
            packages.entry(name).or_insert(dir);
        }
        let workspace = packages
            .iter()
            .map(|(name, dir)| {
                let manifest = dir.join("package.json");
                let exports = read_json(&manifest)
                    .ok()
                    .and_then(|mut json| json.get_mut("exports").map(Value::take))
                    .filter(|exports| !exports.is_null());
                (name.clone(), WorkspaceEntry { manifest, exports })
            })
            .collect();
        let file_system = LinkedFileSystem::new(&root, packages);

        let without_tsconfig = ResolveOptions { tsconfig: None, ..options.clone() };
        let inner = ResolverGeneric::new_with_file_system(file_system, options);
        Ok(Self {
            without_tsconfig: inner.clone_with_options(without_tsconfig),
            inner,
            tsconfig_errors: Mutex::default(),
            root,
            conditions,
            workspace,
            ignore_unresolved,
        })
    }

    /// Resolve `specifier`, as imported by the file at the absolute path
    /// `file`.
    pub fn resolve(&self, file: &Path, specifier: &str) -> Resolved {
        match self.resolve_specifier(file, specifier) {
            Resolved::Unresolved(reason) => {
                if self.ignore_unresolved.iter().any(|pattern| pattern.is_match(specifier)) {
                    Resolved::Ignored(reason)
                } else {
                    Resolved::Unresolved(reason)
                }
            }
            resolved => resolved,
        }
    }

    /// Resolve `specifier`, without applying `ignore_unresolved`.
    fn resolve_specifier(&self, file: &Path, specifier: &str) -> Resolved {
        if let Some((scheme, _)) = split_scheme(specifier) {
            return match scheme {
                "node" | "bun" => Resolved::Builtin(squash_builtin(specifier).to_string()),
                _ => Resolved::unresolved(UnresolvedReason::UnsupportedScheme(scheme.to_string())),
            };
        }

        let error = match self.resolve_file(file, specifier) {
            Ok(resolution) => return self.classify(specifier, resolution.path()),
            Err(ResolveError::Builtin { mut resolved, .. }) => {
                resolved.truncate(squash_builtin(&resolved).len());
                return Resolved::Builtin(resolved);
            }
            // An alias matched, so the specifier doesn't name a package.
            Err(error @ ResolveError::MatchedAliasNotFound(..)) => {
                return failed(error);
            }
            Err(error) => error,
        };

        let (path, query) = split_query(specifier);
        match package_name(path) {
            Some(package) => match self.workspace.get(package) {
                Some(entry) => {
                    let subpath = &path[package.len()..];
                    self.resolve_in_workspace(specifier, package, entry, subpath, query)
                }
                None => Resolved::External { package: package.to_string(), found: false },
            },
            // Blame the `tsconfig.json` if it couldn't be loaded, since it may
            // have had the `paths` that a non-relative specifier needs.
            None if !specifier.starts_with(['.', '/']) => {
                failed(self.tsconfig_error(file).unwrap_or(error))
            }
            None => failed(error),
        }
    }

    /// Resolve `specifier` from `file` with `oxc_resolver`. A file whose
    /// `tsconfig.json` can't be loaded (because it extends one in a package
    /// that isn't installed, say) is resolved without it, and the error is
    /// kept for [`Self::tsconfig_error`]. Since the two resolvers share their
    /// caches, `oxc_resolver` then remembers that the file has no
    /// `tsconfig.json`, instead of failing to load it again for every import.
    fn resolve_file(&self, file: &Path, specifier: &str) -> Result<Resolution, ResolveError> {
        match self.inner.resolve_file(file, specifier) {
            Err(error) if is_tsconfig_error(&error) => {
                let mut errors =
                    self.tsconfig_errors.lock().unwrap_or_else(PoisonError::into_inner);
                if !errors.contains_key(file) {
                    errors.insert(file.to_path_buf(), error);
                }
                drop(errors);
                self.without_tsconfig.resolve_file(file, specifier)
            }
            result => result,
        }
    }

    /// Why the `tsconfig.json` of `file` couldn't be loaded, if it couldn't.
    fn tsconfig_error(&self, file: &Path) -> Option<ResolveError> {
        self.tsconfig_errors.lock().unwrap_or_else(PoisonError::into_inner).get(file).cloned()
    }

    /// Resolve `subpath` (`""` or `"/sub/path"`) of a workspace package
    /// against its directory, through its `exports` map if it has one, and
    /// keep the specifier's `query` (`""`, `"?query"` or `"#fragment"`).
    fn resolve_in_workspace(
        &self,
        specifier: &str,
        package: &str,
        entry: &WorkspaceEntry,
        subpath: &str,
        query: &str,
    ) -> Resolved {
        let target = match &entry.exports {
            Some(exports) => {
                let subpath = format!(".{subpath}");
                match exports::resolve(exports, &subpath, &self.conditions) {
                    Some(target) => target,
                    None => {
                        return Resolved::unresolved(UnresolvedReason::NotExported {
                            package: package.to_string(),
                            subpath,
                        });
                    }
                }
            }
            None if subpath.is_empty() => Cow::Borrowed("."),
            None => Cow::Owned(format!(".{subpath}")),
        };
        let request = match query {
            "" => target,
            query => Cow::Owned(format!("{target}{query}")),
        };
        match self.resolve_file(&entry.manifest, &request) {
            Ok(resolution) => self.classify(specifier, resolution.path()),
            Err(error) => failed(error),
        }
    }

    /// Classify the file that `specifier` resolved to.
    fn classify(&self, specifier: &str, path: &Path) -> Resolved {
        if let Some(relative) = relative_posix(&self.root, path)
            && !relative.split('/').any(|segment| segment == "node_modules")
        {
            return Resolved::Internal(relative);
        }
        let (specifier, _) = split_query(specifier);
        match package_name(specifier).map(str::to_string).or_else(|| package_from_path(path)) {
            Some(package) => Resolved::External { package, found: true },
            None => Resolved::unresolved(UnresolvedReason::OutsideRoot(path.to_path_buf())),
        }
    }
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

        /// The default settings, with the given workspace packages.
        fn with_packages(&self, packages: &[(&str, &str)]) -> ResolverSettings {
            let mut settings = self.settings();
            settings.workspace_packages =
                packages.iter().map(|(name, dir)| (name.to_string(), self.path(dir))).collect();
            settings
        }
    }

    fn internal(path: &str) -> Resolved {
        Resolved::Internal(path.to_string())
    }

    fn external(package: &str, found: bool) -> Resolved {
        Resolved::External { package: package.to_string(), found }
    }

    fn builtin(name: &str) -> Resolved {
        Resolved::Builtin(name.to_string())
    }

    /// Why `resolved` is unresolved, if it is.
    fn unresolved_reason(resolved: &Resolved) -> Option<&UnresolvedReason> {
        match resolved {
            Resolved::Unresolved(reason) => Some(reason),
            _ => None,
        }
    }

    fn is_unresolved(resolved: &Resolved) -> bool {
        matches!(unresolved_reason(resolved), Some(UnresolvedReason::Failed(_)))
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
            unresolved_reason(&resolve("./missing")),
            Some(UnresolvedReason::Failed(failure)) if !failure.is_config_error()
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
        // A path alias that doesn't resolve is not a package.
        assert!(is_unresolved(&resolver.resolve(&web, "@/lib/missing")));
        assert!(is_unresolved(&resolver.resolve(&web, "~config")));
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
    fn files_whose_tsconfig_fails_to_load_resolve_without_it() {
        // Without `node_modules`, the shared config that every package
        // extends can't be found.
        let project = Project::new(&[
            ("packages/tsconfig/package.json", r#"{ "name": "@acme/tsconfig" }"#),
            ("packages/tsconfig/base.json", "{}"),
            (
                "web/tsconfig.json",
                r#"{ "extends": "@acme/tsconfig/base.json", "compilerOptions": { "paths": { "@/*": ["./src/*"] } } }"#,
            ),
            ("web/src/main.ts", ""),
            ("web/src/util.ts", ""),
            ("packages/ui/tsconfig.json", r#"{ "extends": "@acme/tsconfig/base.json" }"#),
            ("packages/ui/package.json", r#"{ "name": "@acme/ui", "exports": "./src/index.ts" }"#),
            ("packages/ui/src/index.ts", ""),
        ]);
        let resolver =
            Resolver::new(project.with_packages(&[("@acme/ui", "packages/ui")])).unwrap();
        let from = project.path("web/src/main.ts");
        let resolve = |specifier: &str| resolver.resolve(&from, specifier);

        assert_eq!(resolve("./util"), internal("web/src/util.ts"));
        assert_eq!(resolve("fs"), builtin("node:fs"));
        assert_eq!(resolve("chart-kit"), external("chart-kit", false));
        assert_eq!(resolve("@acme/ui"), internal("packages/ui/src/index.ts"));
        // The tsconfig's own `paths` are lost with it, and it is to blame,
        // every time.
        for _ in 0..2 {
            let resolved = resolve("@/util");
            assert!(
                matches!(
                    unresolved_reason(&resolved),
                    Some(reason) if reason.is_config_error()
                ),
                "{resolved:?}"
            );
        }
        assert!(is_unresolved(&resolve("./missing")));
    }

    #[test]
    fn tsconfigs_extend_workspace_packages_without_node_modules() {
        let project = Project::new(&[
            ("packages/tsconfig/package.json", r#"{ "name": "@acme/tsconfig" }"#),
            ("packages/tsconfig/base.json", "{}"),
            ("packages/config/package.json", r#"{ "name": "tsconfig" }"#),
            ("packages/config/base.json", "{}"),
            (
                "web/tsconfig.json",
                r#"{ "extends": "@acme/tsconfig/base.json", "compilerOptions": { "paths": { "@/*": ["./src/*"] } } }"#,
            ),
            ("web/src/main.ts", ""),
            ("web/src/util.ts", ""),
            (
                "admin/tsconfig.json",
                r#"{ "extends": "tsconfig/base.json", "compilerOptions": { "paths": { "@/*": ["./app/*"] } } }"#,
            ),
            ("admin/app/main.ts", ""),
            ("admin/app/util.ts", ""),
        ]);
        let resolver = Resolver::new(project.with_packages(&[
            ("@acme/tsconfig", "packages/tsconfig"),
            ("tsconfig", "packages/config"),
        ]))
        .unwrap();

        assert_eq!(
            resolver.resolve(&project.path("web/src/main.ts"), "@/util"),
            internal("web/src/util.ts")
        );
        assert_eq!(
            resolver.resolve(&project.path("admin/app/main.ts"), "@/util"),
            internal("admin/app/util.ts")
        );
    }

    #[test]
    fn workspace_packages_win_over_a_parent_node_modules() {
        // The project is a git worktree inside the main checkout, which has
        // its own copy of each package installed.
        let project = Project::nested(
            "main/.worktrees/wt",
            &[
                ("main/.worktrees/wt/packages/a/src/index.ts", ""),
                (
                    "main/.worktrees/wt/packages/b/package.json",
                    r#"{ "name": "@acme/b", "exports": "./src/index.ts" }"#,
                ),
                ("main/.worktrees/wt/packages/b/src/index.ts", ""),
                (
                    "main/node_modules/@acme/b/package.json",
                    r#"{ "name": "@acme/b", "main": "index.js" }"#,
                ),
                ("main/node_modules/@acme/b/index.js", ""),
                (
                    "main/node_modules/chart-kit/package.json",
                    r#"{ "name": "chart-kit", "main": "index.js" }"#,
                ),
                ("main/node_modules/chart-kit/index.js", ""),
            ],
        );
        let resolver = Resolver::new(project.with_packages(&[("@acme/b", "packages/b")])).unwrap();
        let from = project.path("packages/a/src/index.ts");

        assert_eq!(resolver.resolve(&from, "@acme/b"), internal("packages/b/src/index.ts"));
        // Other packages still resolve where they are installed.
        assert_eq!(resolver.resolve(&from, "chart-kit"), external("chart-kit", true));
    }

    #[test]
    fn installed_packages_win_over_workspace_packages() {
        let project = Project::new(&[
            ("web/main.ts", ""),
            ("packages/ui/package.json", r#"{ "name": "@acme/ui", "exports": "./src/index.ts" }"#),
            ("packages/ui/src/index.ts", ""),
            (
                "packages/api/package.json",
                r#"{ "name": "@acme/api", "exports": "./src/index.ts" }"#,
            ),
            ("packages/api/src/index.ts", ""),
            ("node_modules/@acme/ui/package.json", r#"{ "name": "@acme/ui", "main": "index.js" }"#),
            ("node_modules/@acme/ui/index.js", ""),
        ]);
        let settings =
            project.with_packages(&[("@acme/ui", "packages/ui"), ("@acme/api", "packages/api")]);
        let resolver = Resolver::new(settings).unwrap();
        let from = project.path("web/main.ts");

        assert_eq!(resolver.resolve(&from, "@acme/ui"), external("@acme/ui", true));
        assert_eq!(resolver.resolve(&from, "@acme/api"), internal("packages/api/src/index.ts"));
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
        assert_eq!(error.to_string(), "`ts` is not an extension, such as `.ts`");
    }

    #[test]
    fn resolves_workspace_packages_without_node_modules() {
        let project = Project::new(&[
            ("web/src/main.ts", ""),
            ("packages/api/package.json", r#"{ "name": "@acme/api", "main": "index.ts" }"#),
            ("packages/api/index.ts", ""),
            ("packages/api/src/deep.ts", ""),
            (
                "packages/designSystem/package.json",
                r#"{
                    "name": "@acme/design-system",
                    "exports": {
                        ".": "./index.ts",
                        "./button": { "types": "./src/button.tsx", "default": "./dist/button.js" },
                        "./icons/*": "./src/icons/*.tsx",
                        "./internal/*": null,
                        "./broken": "./src/missing.ts"
                    }
                }"#,
            ),
            ("packages/designSystem/index.ts", ""),
            ("packages/designSystem/src/button.tsx", ""),
            ("packages/designSystem/src/icons/star.tsx", ""),
            ("packages/designSystem/src/icons/outline/star.tsx", ""),
            ("packages/designSystem/src/internal/secret.ts", ""),
        ]);
        let settings = project.with_packages(&[
            ("@acme/api", "packages/api"),
            ("@acme/design-system", "packages/designSystem"),
        ]);
        let resolver = Resolver::new(settings).unwrap();
        let from = project.path("web/src/main.ts");
        let resolve = |specifier: &str| resolver.resolve(&from, specifier);

        // Without `exports`, the main field and any path inside the package.
        assert_eq!(resolve("@acme/api"), internal("packages/api/index.ts"));
        assert_eq!(resolve("@acme/api/src/deep"), internal("packages/api/src/deep.ts"));
        assert_eq!(resolve("@acme/api/src/deep.js"), internal("packages/api/src/deep.ts"));
        assert!(is_unresolved(&resolve("@acme/api/src/missing")));

        // With `exports`, only what it exports.
        assert_eq!(resolve("@acme/design-system"), internal("packages/designSystem/index.ts"));
        assert_eq!(
            resolve("@acme/design-system/button"),
            internal("packages/designSystem/src/button.tsx")
        );
        assert_eq!(
            resolve("@acme/design-system/icons/star"),
            internal("packages/designSystem/src/icons/star.tsx")
        );
        assert_eq!(
            resolve("@acme/design-system/icons/outline/star"),
            internal("packages/designSystem/src/icons/outline/star.tsx")
        );
        let not_exported = |subpath: &str| {
            Resolved::unresolved(UnresolvedReason::NotExported {
                package: "@acme/design-system".to_string(),
                subpath: subpath.to_string(),
            })
        };
        assert_eq!(
            resolve("@acme/design-system/internal/secret"),
            not_exported("./internal/secret")
        );
        assert_eq!(resolve("@acme/design-system/src/button"), not_exported("./src/button"));
        let reason = UnresolvedReason::NotExported {
            package: "@acme/design-system".to_string(),
            subpath: "./src/button".to_string(),
        };
        assert_eq!(reason.to_string(), "`./src/button` is not exported by `@acme/design-system`");
        assert!(is_unresolved(&resolve("@acme/design-system/broken")));
    }

    #[test]
    fn workspace_imports_keep_their_queries() {
        let project = Project::new(&[
            ("web/main.ts", ""),
            (
                "packages/ui/package.json",
                r#"{ "name": "@acme/ui", "exports": { ".": "./src/index.ts", "./styles.css": "./src/styles.css" } }"#,
            ),
            ("packages/ui/src/index.ts", ""),
            ("packages/ui/src/styles.css", ""),
            ("packages/api/package.json", r#"{ "name": "@acme/api", "main": "index.ts" }"#),
            ("packages/api/index.ts", ""),
            ("packages/api/schema.json", "{}"),
        ]);
        let settings =
            project.with_packages(&[("@acme/ui", "packages/ui"), ("@acme/api", "packages/api")]);
        let resolver = Resolver::new(settings).unwrap();
        let from = project.path("web/main.ts");
        let resolve = |specifier: &str| resolver.resolve(&from, specifier);

        assert_eq!(resolve("@acme/ui/styles.css?inline"), internal("packages/ui/src/styles.css"));
        assert_eq!(resolve("@acme/ui?worker"), internal("packages/ui/src/index.ts"));
        assert_eq!(resolve("@acme/api/schema.json?raw"), internal("packages/api/schema.json"));
        assert_eq!(resolve("@acme/api?raw"), internal("packages/api/index.ts"));
        assert_eq!(resolve("chart-kit?url"), external("chart-kit", false));
    }

    #[test]
    fn workspace_exports_use_the_configured_conditions() {
        let project = Project::new(&[
            ("web/main.ts", ""),
            (
                "packages/ui/package.json",
                r#"{ "name": "@acme/ui", "exports": { "source": "./src/index.ts", "types": "./types/index.d.ts", "default": "./dist/index.js" } }"#,
            ),
            ("packages/ui/src/index.ts", ""),
            ("packages/ui/types/index.d.ts", ""),
            ("packages/ui/dist/index.js", ""),
        ]);
        let from = project.path("web/main.ts");

        // The package's order decides between the configured conditions.
        let resolver =
            Resolver::new(project.with_packages(&[("@acme/ui", "packages/ui")])).unwrap();
        assert_eq!(resolver.resolve(&from, "@acme/ui"), internal("packages/ui/src/index.ts"));

        let mut settings = project.with_packages(&[("@acme/ui", "packages/ui")]);
        settings.conditions = vec!["types".to_string()];
        let resolver = Resolver::new(settings).unwrap();
        assert_eq!(resolver.resolve(&from, "@acme/ui"), internal("packages/ui/types/index.d.ts"));

        let mut settings = project.with_packages(&[("@acme/ui", "packages/ui")]);
        settings.conditions = Vec::new();
        let resolver = Resolver::new(settings).unwrap();
        assert_eq!(resolver.resolve(&from, "@acme/ui"), internal("packages/ui/dist/index.js"));
    }

    #[test]
    fn workspace_main_fields_follow_the_settings() {
        let project = Project::new(&[
            ("web/main.ts", ""),
            (
                "packages/ui/package.json",
                r#"{ "name": "@acme/ui", "module": "./src/index.ts", "main": "./dist/index.js" }"#,
            ),
            ("packages/ui/src/index.ts", ""),
            ("packages/ui/dist/index.js", ""),
        ]);
        let from = project.path("web/main.ts");

        let resolver =
            Resolver::new(project.with_packages(&[("@acme/ui", "packages/ui")])).unwrap();
        assert_eq!(resolver.resolve(&from, "@acme/ui"), internal("packages/ui/src/index.ts"));

        let mut settings = project.with_packages(&[("@acme/ui", "packages/ui")]);
        settings.main_fields = vec!["main".to_string()];
        let resolver = Resolver::new(settings).unwrap();
        assert_eq!(resolver.resolve(&from, "@acme/ui"), internal("packages/ui/dist/index.js"));
    }

    #[test]
    fn the_first_workspace_package_with_a_name_wins() {
        let project = Project::new(&[
            ("web/main.ts", ""),
            ("packages/a/package.json", r#"{ "name": "@acme/ui", "main": "a.ts" }"#),
            ("packages/a/a.ts", ""),
            ("packages/b/package.json", r#"{ "name": "@acme/ui", "main": "b.ts" }"#),
            ("packages/b/b.ts", ""),
        ]);
        let settings =
            project.with_packages(&[("@acme/ui", "packages/a"), ("@acme/ui", "packages/b")]);
        let resolver = Resolver::new(settings).unwrap();
        assert_eq!(
            resolver.resolve(&project.path("web/main.ts"), "@acme/ui"),
            internal("packages/a/a.ts")
        );
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

        assert_eq!(resolve("@vendor/sdk"), external("@vendor/sdk", true));
        assert_eq!(resolve("@vendor/sdk/client"), external("@vendor/sdk", true));
        assert_eq!(resolve("chart-kit"), external("chart-kit", true));
        // A relative import into `node_modules` is named from the path.
        assert_eq!(
            resolve("../../node_modules/@vendor/sdk/styles.css"),
            external("@vendor/sdk", true)
        );
        // A subpath that the package doesn't export is still an import of it.
        assert_eq!(resolve("chart-kit/internal"), external("chart-kit", false));
    }

    #[test]
    fn names_packages_that_are_not_installed_from_the_specifier() {
        let project = Project::new(&[("web/src/main.ts", "")]);
        let resolver = Resolver::new(project.settings()).unwrap();
        let from = project.path("web/src/main.ts");

        assert_eq!(resolver.resolve(&from, "@vendor/sdk"), external("@vendor/sdk", false));
        assert_eq!(resolver.resolve(&from, "@vendor/sdk/client"), external("@vendor/sdk", false));
        assert_eq!(resolver.resolve(&from, "chart-kit/lines"), external("chart-kit", false));
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

        assert_eq!(resolver.resolve(&from, "@vendor/sdk"), external("@vendor/sdk", true));
        let outside = resolver.resolve(&from, "../../shared/util");
        let shared = project.root.parent().unwrap().join("shared").join("util.ts");
        assert_eq!(outside, Resolved::unresolved(UnresolvedReason::OutsideRoot(shared)));
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
        assert_eq!(resolve("bun:sqlite"), builtin("bun:sqlite"));
    }

    #[test]
    fn other_schemes_are_unresolved() {
        let project = Project::new(&[("main.ts", "")]);
        let resolver = Resolver::new(project.settings()).unwrap();
        let resolved = resolver.resolve(&project.path("main.ts"), "virtual:pwa-register");
        assert_eq!(
            resolved,
            Resolved::unresolved(UnresolvedReason::UnsupportedScheme("virtual".to_string()))
        );
        if let Resolved::Unresolved(reason) = resolved {
            assert_eq!(reason.to_string(), "unsupported `virtual:` specifier");
        }
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
        // The alias matched, so this is not the `types` package.
        assert!(is_unresolved(&resolver.resolve(&from, "types/missing")));
    }

    #[test]
    fn ignores_expected_unresolved_specifiers() {
        let project = Project::new(&[("web/main.ts", ""), ("web/util.ts", "")]);
        let mut settings = project.settings();
        settings.ignore_unresolved = ["/*", "virtual:*", "~icons/*", "./*"]
            .into_iter()
            .map(|pattern| NamePattern::new(pattern).unwrap())
            .collect();
        let resolver = Resolver::new(settings).unwrap();
        let from = project.path("web/main.ts");
        let resolve = |specifier: &str| resolver.resolve(&from, specifier);

        assert!(matches!(
            resolve("/logo.svg"),
            Resolved::Ignored(reason) if matches!(*reason, UnresolvedReason::Failed(_))
        ));
        assert_eq!(
            resolve("virtual:pwa-register"),
            Resolved::Ignored(Box::new(UnresolvedReason::UnsupportedScheme("virtual".to_string())))
        );
        assert!(matches!(resolve("~icons/mdi/home"), Resolved::Ignored(_)));
        assert!(matches!(resolve("./missing"), Resolved::Ignored(_)));
        // Only unresolved specifiers are ignored.
        assert_eq!(resolve("./util"), internal("web/util.ts"));
        assert!(is_unresolved(&resolve("../missing")));
        assert_eq!(resolve("chart-kit"), external("chart-kit", false));
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
