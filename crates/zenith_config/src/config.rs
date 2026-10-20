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
use zenith_diagnostics::ReportKind;
use zenith_graph::KindSet;
use zenith_utils::{
    fs::{find_in_ancestors, read_text},
    glob::{Glob, NamePattern},
};

use crate::{
    contracts::{ContractConfig, Reexports, Severity},
    error::{ConfigError, LoadError},
    language::{LANGUAGES, Language, LanguageId},
    parse,
};

/// The name of the configuration file.
pub const CONFIG_FILE_NAME: &str = "zenith.toml";

/// The configuration of a zenith project.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Which files to scan for imports.
    pub files: FilesConfig,

    /// The languages to scan, each with its own options.
    pub languages: LanguagesConfig,

    /// Defaults that apply to every contract.
    pub analysis: AnalysisConfig,

    /// The baseline of known violations.
    pub baseline: BaselineConfig,

    /// The cache of what zenith found in each file on earlier runs.
    pub cache: CacheConfig,

    /// The contracts, in the order of the file.
    pub contracts: Vec<ContractConfig>,

    /// The file the configuration was loaded from, so that errors found once
    /// it is used, such as a selector that matches nothing, can be shown in
    /// context. `None` for the defaults, or a configuration parsed from text.
    pub file: Option<ConfigFile>,
}

/// A configuration file, as it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    /// The path it was read from.
    pub path: PathBuf,

    /// Its contents.
    pub contents: String,
}

impl Config {
    /// Find the nearest configuration file in `start` or one of its
    /// ancestors. A relative `start` is taken from the current directory.
    pub fn discover(start: &Path) -> Option<PathBuf> {
        find_in_ancestors(start, CONFIG_FILE_NAME)
    }

    /// Read a configuration file and parse it with [`Config::parse`]. The
    /// configuration keeps the file, in [`Config::file`].
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let contents =
            read_text(path).map_err(|error| LoadError::Read { path: path.to_path_buf(), error })?;
        match Self::parse(&contents) {
            Ok(config) => {
                let file = ConfigFile { path: path.to_path_buf(), contents };
                Ok(Self { file: Some(file), ..config })
            }
            Err(errors) => Err(LoadError::Invalid { path: path.to_path_buf(), contents, errors }),
        }
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

/// Which files to scan for imports, whatever their language.
#[derive(Debug, Clone)]
pub struct FilesConfig {
    /// Globs of files and directories to skip, on top of the default
    /// excludes.
    pub exclude: Vec<Glob>,

    /// Whether to skip the files that `.gitignore`, `.ignore` and the global
    /// git excludes ignore.
    pub respect_gitignore: bool,
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self { exclude: Vec::new(), respect_gitignore: true }
    }
}

/// The languages to scan. A language is enabled when its table is written;
/// without a `[languages]` table, JavaScript and TypeScript are.
#[derive(Debug, Clone)]
pub struct LanguagesConfig {
    /// JavaScript and TypeScript, if enabled.
    pub js: Option<JsConfig>,
}

impl Default for LanguagesConfig {
    fn default() -> Self {
        Self { js: Some(JsConfig::default()) }
    }
}

impl LanguagesConfig {
    /// Whether `language` is enabled.
    pub fn is_enabled(&self, language: &Language) -> bool {
        match language.id {
            LanguageId::Js => self.js.is_some(),
        }
    }

    /// The enabled languages, in the order of the registry.
    pub fn enabled(&self) -> impl Iterator<Item = &'static Language> {
        LANGUAGES.iter().copied().filter(|language| self.is_enabled(language))
    }

    /// The extensions, without the `.`, that a path selector may leave off a
    /// file's path: those of every enabled language.
    pub fn optional_extensions(&self) -> Vec<&'static str> {
        union(self.enabled().map(|language| language.optional_extensions))
    }

    /// Which files to scan: the globs that the enabled languages set as their
    /// `include`, and the source extensions of those that leave it to the
    /// default.
    pub fn scanned(&self) -> Scanned {
        let mut globs = Vec::new();
        let mut extensions = Vec::new();
        for language in self.enabled() {
            let include = match language.id {
                LanguageId::Js => self.js.as_ref().and_then(|js| js.include.as_ref()),
            };
            match include {
                Some(include) => globs.extend(include.iter().cloned()),
                None => extensions.push(language.source_extensions),
            }
        }
        Scanned { globs, extensions: union(extensions) }
    }
}

/// Which files the enabled languages scan: those that match one of the
/// globs, and those with one of the extensions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scanned {
    /// The `include` globs of the languages that set them.
    pub globs: Vec<Glob>,

    /// The source extensions, without the `.`, of the languages that don't.
    pub extensions: Vec<&'static str>,
}

/// The values of `lists`, in order, without repeats.
fn union<'a>(lists: impl IntoIterator<Item = &'a [&'static str]>) -> Vec<&'static str> {
    let mut union: Vec<&'static str> = Vec::new();
    for value in lists.into_iter().flatten() {
        if !union.contains(value) {
            union.push(value);
        }
    }
    union
}

/// The options of JavaScript and TypeScript. `None` options leave the
/// defaults in place.
#[derive(Debug, Clone, Default)]
pub struct JsConfig {
    /// Globs of the files to scan. `None` scans every file with one of the
    /// [`JS`](crate::JS) source extensions.
    pub include: Option<Vec<Glob>>,

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

    /// How contracts see imports through re-exports, unless they say.
    pub reexports: Reexports,
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
pub(crate) const DEFAULT_BASELINE_PATH: &str = "zenith.baseline.toml";

impl BaselineConfig {
    /// How to report the stale entries of a contract of `severity`: as errors
    /// if it is of `error` severity and stale entries are errors, and as
    /// warnings otherwise.
    pub fn stale_kind(&self, severity: Severity) -> ReportKind {
        match (severity, self.stale) {
            (Severity::Error, StaleLevel::Error) => ReportKind::Error,
            _ => ReportKind::Warning,
        }
    }
}

impl Default for BaselineConfig {
    fn default() -> Self {
        Self { path: DEFAULT_BASELINE_PATH.to_string(), stale: StaleLevel::default() }
    }
}

/// The cache of what zenith found in each file on earlier runs, which lets
/// it skip reading and parsing the files that haven't changed since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    /// Whether to use the cache.
    pub enabled: bool,

    /// The directory to keep the cache in, relative to the project root.
    pub dir: String,
}

/// The default directory of the cache.
pub(crate) const DEFAULT_CACHE_DIR: &str = ".zenith_cache";

impl Default for CacheConfig {
    fn default() -> Self {
        Self { enabled: true, dir: DEFAULT_CACHE_DIR.to_string() }
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
    fn discovers_the_nearest_config_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("web/src")).unwrap();
        fs::write(root.join(CONFIG_FILE_NAME), "").unwrap();
        assert_eq!(Config::discover(&root.join("web/src")), Some(root.join(CONFIG_FILE_NAME)));
    }

    #[test]
    fn a_loaded_config_keeps_its_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let contents = "[baseline]\nstale = \"warn\"\n";
        fs::write(&path, contents).unwrap();
        let file = Config::load(&path).unwrap().file.unwrap();
        assert_eq!(file, ConfigFile { path, contents: contents.to_string() });
        assert!(Config::parse(contents).unwrap().file.is_none());
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
