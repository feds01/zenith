//! Specifier resolution.
//!
//! Zenith resolves import specifiers with
//! [`oxc_resolver`](https://crates.io/crates/oxc_resolver), configured to
//! match what TypeScript and bundlers see: the nearest `tsconfig.json` of each
//! file (with `paths`, `extends` and project references), `exports` maps,
//! `.js` to `.ts` remapping and symlinked workspace packages.
//!
//! Every resolution is then classified as an internal file, an external
//! package, a builtin, or unresolved (see [Resolved]).

mod exports;

use std::{
    fs,
    path::{Path, PathBuf},
};

use oxc_resolver::{
    AliasValue, ResolveError, ResolveOptions, TsconfigDiscovery, TsconfigOptions,
    TsconfigReferences,
};
use rustc_hash::FxHashMap;
use serde_json::Value;
use zenith_utils::path::{is_in_node_modules, relative_posix};

/// How to find `tsconfig.json` files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TsconfigMode {
    /// Use the nearest `tsconfig.json` of each file.
    Auto,
    /// Ignore `tsconfig.json` files.
    Off,
    /// Use one `tsconfig.json` for every file.
    File(PathBuf),
}

/// The settings for a [Resolver].
#[derive(Debug, Clone)]
pub struct ResolverSettings {
    /// The project root. Resolutions inside it (outside `node_modules`) are
    /// internal.
    pub root: PathBuf,
    /// How to find `tsconfig.json` files.
    pub tsconfig: TsconfigMode,
    /// Condition names for `exports` and `imports` maps.
    pub conditions: Vec<String>,
    /// `package.json` fields that point at a package's entry point.
    pub main_fields: Vec<String>,
    /// Extensions to try, in order.
    pub extensions: Vec<String>,
    /// Bundler-only aliases: a specifier prefix and the absolute path it
    /// maps to.
    pub aliases: Vec<(String, PathBuf)>,
    /// Workspace package names and their absolute directories. When normal
    /// resolution fails (e.g. `node_modules` is not installed), workspace
    /// imports are resolved directly against these, honouring `exports`.
    pub workspace_packages: Vec<(String, PathBuf)>,
}

impl ResolverSettings {
    /// The default condition names.
    pub fn default_conditions() -> Vec<String> {
        ["types", "source", "import", "module", "require", "node", "default"]
            .map(String::from)
            .to_vec()
    }

    /// The default main fields.
    pub fn default_main_fields() -> Vec<String> {
        ["module", "main"].map(String::from).to_vec()
    }

    /// The default extensions.
    pub fn default_extensions() -> Vec<String> {
        [".ts", ".tsx", ".d.ts", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"]
            .map(String::from)
            .to_vec()
    }

    /// Settings with every default, for the given root.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            tsconfig: TsconfigMode::Auto,
            conditions: Self::default_conditions(),
            main_fields: Self::default_main_fields(),
            extensions: Self::default_extensions(),
            aliases: Vec::new(),
            workspace_packages: Vec::new(),
        }
    }
}

/// The outcome of resolving one specifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A file inside the project, as an absolute (canonical) path.
    Internal(PathBuf),
    /// An external package, by name, e.g. `@mastra/core`. `unresolved` is set
    /// when the package could not be found on disk (e.g. `node_modules` is
    /// not installed) and was classified from the specifier alone.
    External { package: String, unresolved: bool },
    /// A runtime builtin, e.g. `node:fs` or `bun:sqlite`.
    Builtin(String),
    /// The specifier could not be resolved.
    Unresolved(String),
}

/// A resolver for one project. It is cheap to share between threads; its
/// caches are internally synchronised.
pub struct Resolver {
    inner: oxc_resolver::Resolver,
    root: PathBuf,
    conditions: Vec<String>,
    workspace: FxHashMap<String, WorkspaceEntry>,
}

/// A workspace package, for resolution without `node_modules`.
struct WorkspaceEntry {
    dir: PathBuf,
    exports: Option<Value>,
}

impl Resolver {
    /// Create a resolver.
    pub fn new(settings: &ResolverSettings) -> Self {
        let path_alias = |entries: &[(String, PathBuf)]| {
            entries
                .iter()
                .map(|(name, path)| {
                    (name.clone(), vec![AliasValue::Path(path.to_string_lossy().into_owned())])
                })
                .collect()
        };
        let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        let options = ResolveOptions {
            tsconfig: match &settings.tsconfig {
                TsconfigMode::Auto => Some(TsconfigDiscovery::Auto),
                TsconfigMode::Off => None,
                TsconfigMode::File(path) => Some(TsconfigDiscovery::Manual(TsconfigOptions {
                    config_file: path.clone(),
                    references: TsconfigReferences::Auto,
                })),
            },
            alias: path_alias(&settings.aliases),
            condition_names: settings.conditions.clone(),
            main_fields: settings.main_fields.clone(),
            extensions: settings.extensions.clone(),
            extension_alias: vec![
                (".js".into(), strings(&[".ts", ".tsx", ".d.ts", ".js", ".jsx"])),
                (".jsx".into(), strings(&[".tsx", ".jsx"])),
                (".mjs".into(), strings(&[".mts", ".d.mts", ".mjs"])),
                (".cjs".into(), strings(&[".cts", ".d.cts", ".cjs"])),
            ],
            builtin_modules: true,
            symlinks: true,
            ..ResolveOptions::default()
        };

        let workspace = settings
            .workspace_packages
            .iter()
            .map(|(name, dir)| {
                let exports = fs::read_to_string(dir.join("package.json"))
                    .ok()
                    .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
                    .and_then(|json| json.get("exports").cloned());
                (name.clone(), WorkspaceEntry { dir: dir.clone(), exports })
            })
            .collect();

        Self {
            inner: oxc_resolver::Resolver::new(options),
            root: settings.root.clone(),
            conditions: settings.conditions.clone(),
            workspace,
        }
    }

    /// Resolve `specifier` as imported from the absolute path `file`.
    pub fn resolve(&self, file: &Path, specifier: &str) -> Resolved {
        if let Some(builtin) = scheme_builtin(specifier) {
            return builtin;
        }

        match self.inner.resolve_file(file, specifier) {
            Ok(resolution) => self.classify(specifier, resolution.path()),
            Err(ResolveError::Builtin { resolved, .. }) => {
                Resolved::Builtin(squash_builtin(&resolved))
            }
            Err(error) if is_bare(specifier) => match package_name(specifier) {
                Some(package) => match self.workspace.get(package) {
                    Some(entry) => {
                        self.resolve_in_workspace(entry, specifier, &specifier[package.len()..])
                    }
                    None => Resolved::External { package: package.to_string(), unresolved: true },
                },
                None => Resolved::Unresolved(error.to_string()),
            },
            Err(error) => Resolved::Unresolved(error.to_string()),
        }
    }

    /// Resolve `subpath` (`""` or `/sub/path`) of a workspace package directly
    /// against its directory, through its `exports` map if it has one.
    fn resolve_in_workspace(
        &self,
        entry: &WorkspaceEntry,
        specifier: &str,
        subpath: &str,
    ) -> Resolved {
        let request = match &entry.exports {
            Some(exports) => {
                let subpath = format!(".{subpath}");
                match exports::resolve(exports, &subpath, &self.conditions) {
                    Some(target) => target,
                    None => {
                        return Resolved::Unresolved(format!(
                            "`{subpath}` is not exported by the package"
                        ));
                    }
                }
            }
            None if subpath.is_empty() => ".".to_string(),
            None => format!(".{subpath}"),
        };
        match self.inner.resolve_file(entry.dir.join("package.json"), &request) {
            Ok(resolution) => self.classify(specifier, resolution.path()),
            Err(error) => Resolved::Unresolved(error.to_string()),
        }
    }

    fn classify(&self, specifier: &str, path: &Path) -> Resolved {
        match relative_posix(&self.root, path) {
            Some(relative) if !is_in_node_modules(&relative) => {
                Resolved::Internal(path.to_path_buf())
            }
            _ => {
                match package_from_path(path).or_else(|| package_name(specifier).map(String::from))
                {
                    Some(package) => Resolved::External { package, unresolved: false },
                    None => Resolved::Unresolved(format!(
                        "resolved outside the project to {}",
                        path.display()
                    )),
                }
            }
        }
    }
}

/// Handle specifiers with a URL-like scheme before resolving.
fn scheme_builtin(specifier: &str) -> Option<Resolved> {
    let (scheme, rest) = specifier.split_once(':')?;
    if !scheme.chars().all(|c| c.is_ascii_lowercase()) || scheme.is_empty() {
        return None;
    }
    Some(match scheme {
        "node" => Resolved::Builtin(squash_builtin(specifier)),
        "bun" => Resolved::Builtin(format!("bun:{}", rest.split('/').next().unwrap_or(rest))),
        _ => Resolved::Unresolved(format!("unsupported `{scheme}:` specifier")),
    })
}

/// Normalise a builtin to `node:<name>`, without any subpath.
fn squash_builtin(specifier: &str) -> String {
    let name = specifier.strip_prefix("node:").unwrap_or(specifier);
    format!("node:{}", name.split('/').next().unwrap_or(name))
}

/// Whether a specifier is a bare package specifier, rather than a relative
/// or absolute path, a `#` subpath import, or a URL.
pub fn is_bare(specifier: &str) -> bool {
    !(specifier.is_empty()
        || specifier.starts_with('.')
        || specifier.starts_with('/')
        || specifier.starts_with('#')
        || specifier.contains(':'))
}

/// The package name of a bare specifier: `@scope/name/sub` is `@scope/name`,
/// `name/sub` is `name`.
pub fn package_name(specifier: &str) -> Option<&str> {
    if !is_bare(specifier) {
        return None;
    }
    let end = if specifier.starts_with('@') {
        let slash = specifier.find('/')?;
        specifier[slash + 1..].find('/').map_or(specifier.len(), |i| slash + 1 + i)
    } else {
        specifier.find('/').unwrap_or(specifier.len())
    };
    let name = &specifier[..end];
    (!name.is_empty() && name != "@").then_some(name)
}

/// The package a resolved path belongs to, from its last `node_modules`
/// segment.
fn package_from_path(path: &Path) -> Option<String> {
    let parts: Vec<String> =
        path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let index = parts.iter().rposition(|part| part == "node_modules")?;
    let first = parts.get(index + 1)?;
    if first.starts_with('@') {
        Some(format!("{first}/{}", parts.get(index + 2)?))
    } else {
        Some(first.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn extracts_package_names() {
        assert_eq!(package_name("react"), Some("react"));
        assert_eq!(package_name("lodash/get"), Some("lodash"));
        assert_eq!(package_name("@mastra/core"), Some("@mastra/core"));
        assert_eq!(package_name("@mastra/core/agent"), Some("@mastra/core"));
        assert_eq!(package_name("@scope"), None);
        assert_eq!(package_name("./a"), None);
        assert_eq!(package_name("node:fs"), None);
        assert_eq!(package_name("#internal"), None);
    }

    #[test]
    fn extracts_packages_from_paths() {
        let path = Path::new("/r/node_modules/.bun/x/node_modules/@scope/pkg/dist/index.js");
        assert_eq!(package_from_path(path).as_deref(), Some("@scope/pkg"));
        assert_eq!(
            package_from_path(Path::new("/r/node_modules/react/index.js")).as_deref(),
            Some("react")
        );
        assert_eq!(package_from_path(Path::new("/r/src/a.ts")), None);
    }

    /// A tiny project on disk: a workspace package without `node_modules`,
    /// tsconfig paths and `.js` imports of `.ts` files.
    #[test]
    fn resolves_like_typescript() {
        let dir = tempdir();
        let root = dir.as_path();
        let write = |path: &str, contents: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        };
        write("app/tsconfig.json", r#"{ "compilerOptions": { "paths": { "@/*": ["./src/*"] } } }"#);
        write("app/src/main.ts", "");
        write("app/src/lib/util.ts", "");
        write("app/src/view.tsx", "");
        write("packages/api/package.json", r#"{ "name": "@acme/api", "main": "index.ts" }"#);
        write("packages/api/index.ts", "");
        write("packages/api/src/deep.ts", "");
        write(
            "packages/ui/package.json",
            r#"{ "name": "@acme/ui", "exports": { ".": "./index.ts", "./button": { "types": "./src/button.tsx", "default": "./dist/button.js" }, "./icons/*": "./src/icons/*.tsx" } }"#,
        );
        write("packages/ui/index.ts", "");
        write("packages/ui/src/button.tsx", "");
        write("packages/ui/src/icons/star.tsx", "");
        write("packages/ui/src/internal.ts", "");

        let mut settings = ResolverSettings::new(root.to_path_buf());
        settings.workspace_packages = vec![
            ("@acme/api".into(), root.join("packages/api")),
            ("@acme/ui".into(), root.join("packages/ui")),
        ];
        let resolver = Resolver::new(&settings);
        let from = root.join("app/src/main.ts");
        let internal = |path: &str| Resolved::Internal(root.join(path));

        assert_eq!(resolver.resolve(&from, "@/lib/util"), internal("app/src/lib/util.ts"));
        assert_eq!(resolver.resolve(&from, "./lib/util.js"), internal("app/src/lib/util.ts"));
        assert_eq!(resolver.resolve(&from, "./view"), internal("app/src/view.tsx"));
        assert_eq!(resolver.resolve(&from, "@acme/api"), internal("packages/api/index.ts"));
        assert_eq!(
            resolver.resolve(&from, "@acme/api/src/deep"),
            internal("packages/api/src/deep.ts")
        );
        assert_eq!(resolver.resolve(&from, "@acme/ui"), internal("packages/ui/index.ts"));
        assert_eq!(
            resolver.resolve(&from, "@acme/ui/button"),
            internal("packages/ui/src/button.tsx")
        );
        assert_eq!(
            resolver.resolve(&from, "@acme/ui/icons/star"),
            internal("packages/ui/src/icons/star.tsx")
        );
        assert!(matches!(
            resolver.resolve(&from, "@acme/ui/src/internal"),
            Resolved::Unresolved(_)
        ));
        assert_eq!(resolver.resolve(&from, "fs"), Resolved::Builtin("node:fs".into()));
        assert_eq!(
            resolver.resolve(&from, "node:fs/promises"),
            Resolved::Builtin("node:fs".into())
        );
        assert_eq!(resolver.resolve(&from, "bun:sqlite"), Resolved::Builtin("bun:sqlite".into()));
        assert_eq!(
            resolver.resolve(&from, "@mastra/core/agent"),
            Resolved::External { package: "@mastra/core".into(), unresolved: true }
        );
        assert!(matches!(resolver.resolve(&from, "./missing"), Resolved::Unresolved(_)));
        assert!(matches!(resolver.resolve(&from, "virtual:pwa"), Resolved::Unresolved(_)));
    }

    /// A unique, canonical temporary directory.
    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zenith-resolve-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        zenith_utils::path::adjust_canonicalisation(dir)
    }
}
