//! Contract configuration types.

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_graph::ImportKind;

/// One architecture contract. The `type` key selects the kind of contract.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContractConfig {
    /// Higher layers may import lower layers, never the reverse.
    Layers(LayersConfig),
    /// The `from` modules must not import the `to` modules.
    Forbidden(ForbiddenConfig),
    /// The modules must not depend on one another.
    Independence(IndependenceConfig),
    /// Only the allowed importers may directly import the modules.
    Protected(ProtectedConfig),
    /// No dependency cycles between the modules.
    Acyclic(AcyclicConfig),
    /// The modules may only be imported through their entry points.
    Encapsulation(EncapsulationConfig),
}

impl ContractConfig {
    /// The options shared by every contract type.
    pub fn common(&self) -> &CommonOptions {
        match self {
            ContractConfig::Layers(c) => &c.common,
            ContractConfig::Forbidden(c) => &c.common,
            ContractConfig::Independence(c) => &c.common,
            ContractConfig::Protected(c) => &c.common,
            ContractConfig::Acyclic(c) => &c.common,
            ContractConfig::Encapsulation(c) => &c.common,
        }
    }

    /// The name of the contract type, as written in `type`.
    pub fn type_name(&self) -> &'static str {
        match self {
            ContractConfig::Layers(_) => "layers",
            ContractConfig::Forbidden(_) => "forbidden",
            ContractConfig::Independence(_) => "independence",
            ContractConfig::Protected(_) => "protected",
            ContractConfig::Acyclic(_) => "acyclic",
            ContractConfig::Encapsulation(_) => "encapsulation",
        }
    }
}

/// How severe a broken contract is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A broken contract fails the check.
    #[default]
    Error,
    /// A broken contract is reported without failing the check.
    Warn,
}

/// How to report an `ignore_imports` entry that matches nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AlertLevel {
    /// Fail the contract.
    #[default]
    Error,
    /// Report a warning.
    Warn,
    /// Say nothing.
    None,
}

/// A selector of modules: a path glob (`web/src/domains/*`), a workspace
/// package (`pkg:@acme/*`), an external package (`npm:react`), a builtin
/// (`node:*`), or a table for the advanced forms.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SelectorConfig {
    /// A string selector.
    Pattern(String),
    /// A selector with exclusions or a directive filter.
    Table(SelectorTable),
}

/// The table form of a selector.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectorTable {
    /// Any string selector.
    pub path: String,

    /// Selectors whose members are removed from each matched module.
    #[serde(default)]
    pub except: Vec<String>,

    /// Keep only files with this directive in their prologue, e.g.
    /// `"use client"`.
    #[serde(default)]
    pub directive: Option<String>,
}

/// A layer of a `layers` contract: `"a"`, `"a | b"` (independent
/// siblings), `"a : b"` (siblings that may import each other) or `"(a)"`
/// (optional), or a table to mark the layer as closed.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum LayerConfig {
    /// A layer string.
    Layer(String),
    /// A layer with options.
    Table {
        /// The layer string.
        layer: String,
        /// A closed layer may not be bypassed: layers above it may not
        /// import layers below it.
        #[serde(default)]
        closed: bool,
    },
}

/// Options shared by every contract type.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct CommonOptions {
    /// A human-readable title for output. Defaults to the id.
    #[serde(default)]
    pub name: Option<String>,

    /// `"warn"` reports the contract without failing the check.
    #[serde(default)]
    pub severity: Severity,

    /// Import kinds to ignore, e.g. `["type"]`. Defaults to
    /// `[analysis] exclude_kinds`.
    #[serde(default)]
    pub exclude_kinds: Option<Vec<ImportKind>>,

    /// Imports to ignore, as `"<selector> -> <selector>"`.
    #[serde(default)]
    pub ignore_imports: Vec<String>,

    /// How to report `ignore_imports` entries that match nothing.
    #[serde(default)]
    pub unmatched_ignore_imports: AlertLevel,

    /// Text printed under the violations, e.g. how to fix them.
    #[serde(default)]
    pub guidance: Option<String>,
}

/// Declares a contract config struct: the shared options plus the given
/// type-specific fields. The shared options are spelled out in each struct
/// (rather than flattened) so that unknown keys are still rejected.
macro_rules! contract_config {
    (
        $(#[$meta:meta])*
        $name:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident: $ty:ty
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, JsonSchema)]
        pub struct $name {
            /// Options shared by every contract type.
            #[schemars(flatten)]
            pub common: CommonOptions,
            $(
                $(#[$field_meta])*
                pub $field: $ty,
            )*
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Raw {
                    #[serde(default)]
                    name: Option<String>,
                    #[serde(default)]
                    severity: Severity,
                    #[serde(default)]
                    exclude_kinds: Option<Vec<ImportKind>>,
                    #[serde(default)]
                    ignore_imports: Vec<String>,
                    #[serde(default)]
                    unmatched_ignore_imports: AlertLevel,
                    #[serde(default)]
                    guidance: Option<String>,
                    $(
                        $(#[$field_meta])*
                        $field: $ty,
                    )*
                }

                let raw = Raw::deserialize(deserializer)?;
                Ok($name {
                    common: CommonOptions {
                        name: raw.name,
                        severity: raw.severity,
                        exclude_kinds: raw.exclude_kinds,
                        ignore_imports: raw.ignore_imports,
                        unmatched_ignore_imports: raw.unmatched_ignore_imports,
                        guidance: raw.guidance,
                    },
                    $($field: raw.$field,)*
                })
            }
        }
    };
}

contract_config! {
    /// A `layers` contract.
    LayersConfig {
        /// The layers, ordered from highest to lowest.
        layers: Vec<LayerConfig>,
        /// If set, layers are relative to each container, and each container
        /// is checked separately.
        #[serde(default)]
        containers: Vec<SelectorConfig>,
        /// Every child of every container must be declared as a layer.
        #[serde(default)]
        exhaustive: bool,
        /// Child names exempt from the exhaustiveness check.
        #[serde(default)]
        exhaustive_ignores: Vec<String>,
        /// Only check direct imports.
        #[serde(default)]
        allow_indirect_imports: bool,
    }
}

contract_config! {
    /// A `forbidden` contract.
    ForbiddenConfig {
        /// The modules that must not import the `to` modules.
        from: Vec<SelectorConfig>,
        /// The modules that must not be imported. May include `npm:` and
        /// `node:` selectors.
        to: Vec<SelectorConfig>,
        /// Only check direct imports.
        #[serde(default)]
        allow_indirect_imports: bool,
    }
}

contract_config! {
    /// An `independence` contract.
    IndependenceConfig {
        /// The modules that must not depend on one another.
        modules: Vec<SelectorConfig>,
        /// Only check direct imports.
        #[serde(default)]
        allow_indirect_imports: bool,
    }
}

contract_config! {
    /// A `protected` contract.
    ProtectedConfig {
        /// The protected modules.
        modules: Vec<SelectorConfig>,
        /// The only modules that may directly import the protected modules.
        allowed_importers: Vec<SelectorConfig>,
    }
}

contract_config! {
    /// An `acyclic` contract.
    AcyclicConfig {
        /// The modules that may not form dependency cycles.
        modules: Vec<SelectorConfig>,
        /// How many further generations of children to check.
        #[serde(default)]
        depth: u32,
        /// Modules whose children are not checked.
        #[serde(default)]
        skip: Vec<SelectorConfig>,
    }
}

contract_config! {
    /// An `encapsulation` contract.
    EncapsulationConfig {
        /// The encapsulated modules (directories).
        modules: Vec<SelectorConfig>,
        /// Globs, relative to each module, of the files that may be imported
        /// from outside it. Defaults to `["index"]`.
        #[serde(default)]
        entrypoints: Option<Vec<String>>,
        /// Modules that may import anything.
        #[serde(default)]
        allowed_importers: Vec<SelectorConfig>,
    }
}
