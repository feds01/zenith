//! The `zenith.toml` configuration file: its types, discovery and parsing.
//!
//! The types here mirror the file one-to-one and are deliberately "raw":
//! selectors and globs are kept as strings, and are compiled and validated
//! against the project by `zenith_contracts`. A JSON Schema of the file is
//! generated from these types with [json_schema].

mod contracts;

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

pub use contracts::*;
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::Deserialize;
pub use zenith_graph::ImportKind;

/// The name of the configuration file.
pub const CONFIG_FILE_NAME: &str = "zenith.toml";

/// The configuration of a zenith project.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Which files to scan for imports.
    #[serde(default)]
    pub files: FilesConfig,

    /// How import specifiers are resolved.
    #[serde(default)]
    pub resolve: ResolveConfig,

    /// Defaults that apply to every contract.
    #[serde(default)]
    pub analysis: AnalysisConfig,

    /// The baseline of known violations.
    #[serde(default)]
    pub baseline: BaselineConfig,

    /// The architecture contracts, keyed by id. Ids are used by
    /// `--contract`, the baseline and `zenith-ignore` comments.
    #[serde(default)]
    pub contracts: IndexMap<String, ContractConfig>,
}

/// Which files to scan for imports.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilesConfig {
    /// Globs of files to scan, relative to the project root. Defaults to
    /// every JavaScript and TypeScript file.
    #[serde(default)]
    pub include: Option<Vec<String>>,

    /// Globs of files and directories to skip, in addition to the default
    /// excludes (`node_modules`, `dist`, `.next`, ...).
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Whether to skip files ignored by `.gitignore`, `.ignore` and the
    /// global git excludes.
    #[serde(default = "default_true")]
    pub respect_gitignore: bool,
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self { include: None, exclude: Vec::new(), respect_gitignore: true }
    }
}

/// How import specifiers are resolved.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveConfig {
    /// `"auto"` (the default) uses the nearest `tsconfig.json` of each file,
    /// `"off"` ignores them, and any other value is the path of a single
    /// `tsconfig.json` to use for every file.
    #[serde(default)]
    pub tsconfig: Option<String>,

    /// Condition names used for `exports` maps, in addition to nothing: when
    /// set, this replaces the default list.
    #[serde(default)]
    pub conditions: Option<Vec<String>>,

    /// `package.json` fields that point at a package's entry point.
    #[serde(default)]
    pub main_fields: Option<Vec<String>>,

    /// Extensions to try when a specifier has none, in order.
    #[serde(default)]
    pub extensions: Option<Vec<String>>,

    /// Bundler-only aliases: a specifier prefix and the path, relative to
    /// the project root, that it maps to.
    #[serde(default)]
    pub alias: IndexMap<String, String>,

    /// Specifier globs that are expected not to resolve, e.g. `"/*"` for
    /// files served from a public directory, or `"virtual:*"`.
    #[serde(default)]
    pub ignore_unresolved: Vec<String>,
}

/// Defaults that apply to every contract.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConfig {
    /// Import kinds that contracts ignore unless they set their own
    /// `exclude_kinds`.
    #[serde(default)]
    pub exclude_kinds: Vec<ImportKind>,
}

/// The baseline of known violations.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaselineConfig {
    /// The path of the baseline file, relative to the project root.
    #[serde(default = "default_baseline_path")]
    pub path: String,

    /// How to report baseline entries whose import no longer exists.
    #[serde(default)]
    pub stale: StaleLevel,
}

impl Default for BaselineConfig {
    fn default() -> Self {
        Self { path: default_baseline_path(), stale: StaleLevel::default() }
    }
}

/// How to report stale baseline entries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StaleLevel {
    /// Stale entries fail the check, so the baseline can only shrink.
    #[default]
    Error,
    /// Stale entries are reported as warnings.
    Warn,
}

fn default_true() -> bool {
    true
}

fn default_baseline_path() -> String {
    "zenith.baseline.json".to_string()
}

/// The same as [Config], but with each contract left unparsed so that
/// errors can name the contract they are in.
#[derive(Deserialize)]
struct RawConfig {
    #[serde(default)]
    files: FilesConfig,
    #[serde(default)]
    resolve: ResolveConfig,
    #[serde(default)]
    analysis: AnalysisConfig,
    #[serde(default)]
    baseline: BaselineConfig,
    #[serde(default)]
    contracts: IndexMap<String, toml::Value>,
}

/// An error in a configuration file.
#[derive(Debug, Clone)]
pub struct ConfigError {
    /// The configuration file.
    pub path: PathBuf,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid configuration in `{}`: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Find the configuration file by searching upwards from `start`.
pub fn discover(start: &Path) -> Option<PathBuf> {
    start.ancestors().map(|dir| dir.join(CONFIG_FILE_NAME)).find(|path| path.is_file())
}

/// Read and parse a configuration file.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let contents = fs::read_to_string(path)
        .map_err(|error| ConfigError { path: path.to_path_buf(), message: error.to_string() })?;
    parse(&contents).map_err(|message| ConfigError { path: path.to_path_buf(), message })
}

/// Parse the contents of a configuration file.
pub fn parse(contents: &str) -> Result<Config, String> {
    let raw: RawConfig =
        toml::from_str(contents).map_err(|error| error.to_string().trim().to_string())?;

    let mut contracts = IndexMap::new();
    for (id, value) in raw.contracts {
        if !is_valid_id(&id) {
            return Err(format!(
                "invalid contract id `{id}`: ids may only contain letters, digits, `-` and `_`"
            ));
        }
        let contract = ContractConfig::deserialize(value)
            .map_err(|error| format!("in contract `{id}`: {}", error.to_string().trim()))?;
        contracts.insert(id, contract);
    }

    Ok(Config {
        files: raw.files,
        resolve: raw.resolve,
        analysis: raw.analysis,
        baseline: raw.baseline,
        contracts,
    })
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The JSON Schema of `zenith.toml`.
pub fn json_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Config)).expect("the schema serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_config() {
        let config = parse(
            r#"
[files]
exclude = ["experimental/**"]

[resolve]
alias = { "types" = "web/src/types" }

[analysis]
exclude_kinds = ["type"]

[contracts.client-safety]
type = "forbidden"
from = [{ path = "web/src/**", directive = "use client" }]
to = ["npm:sharp", "node:*"]
guidance = "Keep it out."

[contracts.tiers]
type = "layers"
layers = ["apps/* | services/*", { layer = "pkg:@acme/api", closed = true }, "pkg:@acme/utils"]
severity = "warn"
"#,
        )
        .unwrap();

        assert_eq!(config.files.exclude, vec!["experimental/**"]);
        assert!(config.files.respect_gitignore);
        assert_eq!(config.analysis.exclude_kinds, vec![ImportKind::Type]);
        assert_eq!(config.baseline.path, "zenith.baseline.json");
        let ids: Vec<_> = config.contracts.keys().collect();
        assert_eq!(ids, vec!["client-safety", "tiers"]);

        let ContractConfig::Forbidden(forbidden) = &config.contracts["client-safety"] else {
            panic!("expected a forbidden contract");
        };
        assert_eq!(forbidden.to.len(), 2);
        assert!(
            matches!(&forbidden.from[0], SelectorConfig::Table(t) if t.directive.as_deref() == Some("use client"))
        );

        let ContractConfig::Layers(layers) = &config.contracts["tiers"] else {
            panic!("expected a layers contract");
        };
        assert_eq!(layers.common.severity, Severity::Warn);
        assert!(matches!(&layers.layers[1], LayerConfig::Table { closed: true, .. }));
    }

    #[test]
    fn errors_name_the_contract() {
        let error = parse(
            r#"
[contracts.oops]
type = "forbidden"
fromm = ["a"]
to = ["b"]
"#,
        )
        .unwrap_err();
        assert!(error.contains("in contract `oops`"), "{error}");
        assert!(error.contains("fromm"), "{error}");

        let error = parse("[contracts.x]\ntype = \"layer\"\n").unwrap_err();
        assert!(error.contains("layers"), "{error}");

        let error = parse("[files]\nexclud = []\n").unwrap_err();
        assert!(error.contains("exclud"), "{error}");
    }

    #[test]
    fn generates_a_schema() {
        let schema = json_schema();
        assert!(schema.to_string().contains("allow_indirect_imports"));
    }
}
