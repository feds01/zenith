//! The validated configuration of a zenith project, and finding and loading
//! its file.
//!
//! These types are what [`Config::parse`] produces: defaults are filled in,
//! and selectors, globs, layers and ignored imports are checked and compiled.
//! Resolving selectors against the project is left to the contracts.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zenith_graph::KindSet;
use zenith_utils::{
    fs::{find_in_ancestors, read_text},
    glob::{Glob, NamePattern},
};

use crate::{
    contracts::ContractConfig,
    error::{ConfigError, LoadError},
    parse,
};

/// The name of the configuration file.
pub const CONFIG_FILE_NAME: &str = "zenith.toml";

/// The configuration of a zenith project.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Which files to scan for imports.
    pub files: FilesConfig,

    /// How import specifiers are resolved.
    pub resolve: ResolveConfig,

    /// Defaults that apply to every contract.
    pub analysis: AnalysisConfig,

    /// The baseline of known violations.
    pub baseline: BaselineConfig,

    /// The contracts, in the order of the file.
    pub contracts: Vec<ContractConfig>,
}

impl Config {
    /// Find the nearest configuration file in `start` or one of its
    /// ancestors. A relative `start` is taken from the current directory.
    pub fn discover(start: &Path) -> Option<PathBuf> {
        find_in_ancestors(start, CONFIG_FILE_NAME)
    }

    /// Read a configuration file and parse it with [`Config::parse`].
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let contents =
            read_text(path).map_err(|error| LoadError::Read { path: path.to_path_buf(), error })?;
        Self::parse(&contents).map_err(|errors| LoadError::Invalid {
            path: path.to_path_buf(),
            contents,
            errors,
        })
    }

    /// Parse and validate the contents of a configuration file. On failure,
    /// returns the errors, in the order of the file: the first error in the
    /// structure of each table and contract, such as an unknown key, and
    /// every invalid value, such as a selector.
    pub fn parse(contents: &str) -> Result<Self, Vec<ConfigError>> {
        parse::parse(contents)
    }

    /// The contract with the given id.
    pub fn contract(&self, id: &str) -> Option<&ContractConfig> {
        self.contracts.iter().find(|contract| contract.id == id)
    }
}

/// The extensions, without the `.`, of the JavaScript and TypeScript files
/// that zenith scans when `[files] include` is not set. Declaration files
/// (`.d.ts`) are `ts` files.
pub const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// The extensions, without the `.`, that a path selector may leave off a
/// file's path, so `src/types` matches the file `src/types.ts` as well as the
/// directory `src/types`.
pub const OPTIONAL_EXTENSIONS: &[&str] =
    &["ts", "tsx", "mts", "cts", "d.ts", "js", "jsx", "mjs", "cjs"];

/// Which files to scan for imports.
#[derive(Debug, Clone)]
pub struct FilesConfig {
    /// Globs of the files to scan. `None` scans every file with one of the
    /// [`SOURCE_EXTENSIONS`].
    pub include: Option<Vec<Glob>>,

    /// Globs of files and directories to skip, on top of the default
    /// excludes.
    pub exclude: Vec<Glob>,

    /// Whether to skip the files that `.gitignore`, `.ignore` and the global
    /// git excludes ignore.
    pub respect_gitignore: bool,
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self { include: None, exclude: Vec::new(), respect_gitignore: true }
    }
}

/// How import specifiers are resolved. `None` options leave the resolver's
/// defaults in place.
#[derive(Debug, Clone, Default)]
pub struct ResolveConfig {
    /// Which `tsconfig.json` files to use.
    pub tsconfig: Tsconfig,

    /// The condition names to match in `exports` maps, replacing the
    /// defaults.
    pub conditions: Option<Vec<String>>,

    /// The `package.json` fields that point at a package's entry point,
    /// replacing the defaults.
    pub main_fields: Option<Vec<String>>,

    /// The extensions to try when a specifier has none, in order, replacing
    /// the defaults.
    pub extensions: Option<Vec<String>>,

    /// Bundler-only aliases, in the order of the file: a specifier prefix,
    /// and the path relative to the project root that it maps to.
    pub alias: IndexMap<String, String>,

    /// Patterns of specifiers that are expected not to resolve, so are not
    /// reported as unresolved.
    pub ignore_unresolved: Vec<NamePattern>,
}

/// Which `tsconfig.json` files the resolver uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Tsconfig {
    /// Each file uses its nearest `tsconfig.json`.
    #[default]
    Auto,

    /// No `tsconfig.json` is used.
    Off,

    /// Every file uses this `tsconfig.json`, a path relative to the project
    /// root.
    File(String),
}

/// Defaults that apply to every contract.
#[derive(Debug, Clone, Default)]
pub struct AnalysisConfig {
    /// The import kinds that contracts ignore, unless they set their own.
    pub exclude_kinds: KindSet,
}

/// The baseline of known violations.
#[derive(Debug, Clone)]
pub struct BaselineConfig {
    /// The path of the baseline file, relative to the project root.
    pub path: String,

    /// How to report baseline entries whose import no longer exists.
    pub stale: StaleLevel,
}

/// The default path of the baseline file.
pub(crate) const DEFAULT_BASELINE_PATH: &str = "zenith.baseline.json";

impl Default for BaselineConfig {
    fn default() -> Self {
        Self { path: DEFAULT_BASELINE_PATH.to_string(), stale: StaleLevel::default() }
    }
}

/// How to report baseline entries whose import no longer exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StaleLevel {
    /// Stale entries fail the check, so the baseline can only shrink.
    #[default]
    Error,

    /// Stale entries are reported as warnings.
    Warn,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn selectors_may_leave_off_every_source_extension() {
        for extension in SOURCE_EXTENSIONS {
            assert!(OPTIONAL_EXTENSIONS.contains(extension), "{extension}");
        }
    }

    #[test]
    fn discovers_the_nearest_config_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("web/src")).unwrap();
        fs::write(root.join(CONFIG_FILE_NAME), "").unwrap();
        assert_eq!(Config::discover(&root.join("web/src")), Some(root.join(CONFIG_FILE_NAME)));
    }

    #[test]
    fn loads_a_config_with_a_byte_order_mark() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(&path, "\u{feff}[baseline]\nstale = \"warn\"\n").unwrap();
        assert_eq!(Config::load(&path).unwrap().baseline.stale, StaleLevel::Warn);
    }

    #[test]
    fn tells_unreadable_from_invalid_configs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let error = Config::load(&path).unwrap_err();
        assert!(matches!(&error, LoadError::Read { path: p, .. } if *p == path), "{error:?}");
        assert!(error.to_string().starts_with("could not read `"), "{error}");

        let contents = "[files]\nexclud = []\n\n[contracts.a]\ntype = \"layer\"\n";
        fs::write(&path, contents).unwrap();
        let error = Config::load(&path).unwrap_err();
        let LoadError::Invalid { contents: read, errors, .. } = &error else {
            panic!("expected an invalid config, got {error:?}");
        };
        assert_eq!(read, contents);
        assert_eq!(errors.len(), 2);

        // The errors are shown in context.
        let path = path.display();
        let expected = [
            "error: unknown key `exclud`".to_string(),
            format!(" --> {path}:2:1"),
            "  |".to_string(),
            "2 | exclud = []".to_string(),
            "  | ^^^^^^".to_string(),
            "  |".to_string(),
            "  = help: did you mean `exclude`?".to_string(),
            String::new(),
            "error: unknown contract type `layer`".to_string(),
            format!(" --> {path}:5:8"),
            "  |".to_string(),
            "5 | type = \"layer\"".to_string(),
            "  |        ^^^^^^^".to_string(),
            "  |".to_string(),
            "  = note: in contract `a`".to_string(),
            "  = help: did you mean `layers`?".to_string(),
        ];
        assert_eq!(error.to_string(), expected.join("\n"));
    }
}
