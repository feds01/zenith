//! The settings of a [`Resolver`](crate::Resolver), and their defaults.

use std::path::PathBuf;

/// The default condition names for `exports` and `imports` maps. `types` is
/// included so that internal TypeScript packages resolve to their sources,
/// not to their build output.
pub const DEFAULT_CONDITIONS: &[&str] =
    &["types", "source", "import", "module", "require", "node", "default"];

/// The default `package.json` fields that point at a package's entry point.
pub const DEFAULT_MAIN_FIELDS: &[&str] = &["module", "main"];

/// The default extensions to try, in order, for a specifier without one.
pub const DEFAULT_EXTENSIONS: &[&str] =
    &[".ts", ".tsx", ".d.ts", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"];

/// How to find the `tsconfig.json` of each file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TsconfigMode {
    /// Use the nearest `tsconfig.json` that includes the file, with its
    /// `extends` and project references.
    #[default]
    Auto,

    /// Ignore `tsconfig.json` files, so `paths` and `baseUrl` never apply.
    Off,

    /// Use this `tsconfig.json`, given as an absolute path, for every file,
    /// with its `extends` and project references.
    File(PathBuf),
}

/// The settings for a [`Resolver`](crate::Resolver).
#[derive(Debug, Clone)]
pub struct ResolverSettings {
    /// The canonical project root. A file inside it, and not under
    /// `node_modules`, is internal.
    pub root: PathBuf,

    /// How to find the `tsconfig.json` of each file.
    pub tsconfig: TsconfigMode,

    /// The condition names for `exports` and `imports` maps. A package's
    /// conditions are matched in the order the package lists them, so this
    /// order doesn't matter.
    pub conditions: Vec<String>,

    /// The `package.json` fields that point at a package's entry point, in
    /// order of preference.
    pub main_fields: Vec<String>,

    /// The extensions to try, in order, for a specifier without one. Each one
    /// starts with `.`.
    pub extensions: Vec<String>,

    /// Bundler-only aliases: a specifier, and the absolute path it stands
    /// for. An alias covers the specifier's subpaths too, so an alias of
    /// `types` to `web/src/types` maps `types/user` to `web/src/types/user`.
    pub aliases: Vec<(String, PathBuf)>,
}

impl ResolverSettings {
    /// The default settings for the project at `root`, which must be
    /// canonical.
    pub fn new(root: PathBuf) -> Self {
        let strings = |items: &[&str]| items.iter().map(|item| item.to_string()).collect();
        Self {
            root,
            tsconfig: TsconfigMode::Auto,
            conditions: strings(DEFAULT_CONDITIONS),
            main_fields: strings(DEFAULT_MAIN_FIELDS),
            extensions: strings(DEFAULT_EXTENSIONS),
            aliases: Vec::new(),
        }
    }
}
