//! The validated configuration of a zenith project.
//!
//! These types are what [`parse`](crate::parse) produces: defaults are filled
//! in, and selectors, globs, layers and ignored imports are checked and
//! compiled. Resolving selectors against the project is left to the
//! contracts.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zenith_graph::KindSet;
use zenith_utils::glob::{Glob, NamePattern};

use crate::{
    layer::Layer,
    selector::{IgnoreImport, Selector},
};

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
    /// The contract with the given id.
    pub fn contract(&self, id: &str) -> Option<&ContractConfig> {
        self.contracts.iter().find(|contract| contract.id == id)
    }
}

/// Which files to scan for imports.
#[derive(Debug, Clone)]
pub struct FilesConfig {
    /// Globs of the files to scan. `None` scans every JavaScript and
    /// TypeScript file.
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

/// How severe a broken contract is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A broken contract fails the check.
    #[default]
    Error,

    /// A broken contract is reported without failing the check.
    Warn,
}

/// How to report an `ignore_imports` entry that matches no import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AlertLevel {
    /// The contract is broken.
    #[default]
    Error,

    /// A warning is reported.
    Warn,

    /// Nothing is reported.
    None,
}

/// A contract, with the options that every contract type shares.
#[derive(Debug, Clone)]
pub struct ContractConfig {
    /// The id, which `--contract`, the baseline and `zenith-ignore` comments
    /// refer to.
    pub id: String,

    /// The title shown in output. Defaults to the id.
    pub name: String,

    /// How severe it is when the contract is broken.
    pub severity: Severity,

    /// The import kinds the contract ignores: its own `exclude_kinds`, or
    /// else `[analysis] exclude_kinds`.
    pub exclude_kinds: KindSet,

    /// Imports to remove before checking, in the order of the file.
    pub ignore_imports: Vec<IgnoreImport>,

    /// How to report an `ignore_imports` entry that matches no import.
    pub unmatched_ignore_imports: AlertLevel,

    /// Text shown under the violations, e.g. how to fix them.
    pub guidance: Option<String>,

    /// The contract type and its own options.
    pub kind: ContractKind,
}

/// A contract type, with the options of that type.
#[derive(Debug, Clone)]
pub enum ContractKind {
    /// The `from` modules must not import the `to` modules.
    Forbidden(ForbiddenConfig),

    /// The modules must not depend on one another.
    Independence(IndependenceConfig),

    /// Higher layers may import lower layers, never the reverse.
    Layers(LayersConfig),

    /// Only the allowed importers may directly import the modules.
    Protected(ProtectedConfig),

    /// The modules must not form dependency cycles.
    Acyclic(AcyclicConfig),

    /// The modules may only be imported through their entry points.
    Encapsulation(EncapsulationConfig),
}

impl ContractKind {
    /// The name of the contract type, as written in `type`.
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Forbidden(_) => "forbidden",
            Self::Independence(_) => "independence",
            Self::Layers(_) => "layers",
            Self::Protected(_) => "protected",
            Self::Acyclic(_) => "acyclic",
            Self::Encapsulation(_) => "encapsulation",
        }
    }
}

/// The options of a `forbidden` contract.
#[derive(Debug, Clone)]
pub struct ForbiddenConfig {
    /// The modules that must not import the `to` modules.
    pub from: Vec<Selector>,

    /// The modules that must not be imported.
    pub to: Vec<Selector>,

    /// Whether only direct imports count, rather than chains of imports.
    pub allow_indirect_imports: bool,
}

/// The options of an `independence` contract.
#[derive(Debug, Clone)]
pub struct IndependenceConfig {
    /// The modules that must not depend on one another.
    pub modules: Vec<Selector>,

    /// Whether only direct imports count, rather than chains of imports.
    pub allow_indirect_imports: bool,
}

/// The options of a `layers` contract.
#[derive(Debug, Clone)]
pub struct LayersConfig {
    /// The layers, from highest to lowest. There is at least one. With
    /// `containers`, every member is a path selector relative to each
    /// container.
    pub layers: Vec<Layer>,

    /// The containers that the layers are relative to, each checked on its
    /// own. Empty when the layers are relative to the project root.
    pub containers: Vec<Selector>,

    /// Whether every child of every container must be declared as a layer.
    /// Only set when there are containers.
    pub exhaustive: bool,

    /// The children exempt from the exhaustiveness check, matched against
    /// the base name with or without its JS/TS extension.
    pub exhaustive_ignores: Vec<NamePattern>,

    /// Whether only direct imports count, rather than chains of imports.
    pub allow_indirect_imports: bool,
}

/// The options of a `protected` contract.
#[derive(Debug, Clone)]
pub struct ProtectedConfig {
    /// The protected modules.
    pub modules: Vec<Selector>,

    /// The only modules that may directly import the protected modules,
    /// besides the protected modules themselves.
    pub allowed_importers: Vec<Selector>,
}

/// The options of an `acyclic` contract.
#[derive(Debug, Clone)]
pub struct AcyclicConfig {
    /// The modules that must not form dependency cycles.
    pub modules: Vec<Selector>,

    /// How many further generations of children to check as their own sets
    /// of modules.
    pub depth: u32,

    /// Modules whose children are not checked.
    pub skip: Vec<Selector>,
}

/// The options of an `encapsulation` contract.
#[derive(Debug, Clone)]
pub struct EncapsulationConfig {
    /// The encapsulated modules.
    pub modules: Vec<Selector>,

    /// Globs, relative to each module, of the files that may be imported from
    /// outside it. Defaults to `index`.
    pub entrypoints: Vec<Glob>,

    /// Modules that may import any file of the encapsulated modules.
    pub allowed_importers: Vec<Selector>,
}
