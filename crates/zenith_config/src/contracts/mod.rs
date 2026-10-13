//! The validated contracts of a configuration: the options that every
//! contract type shares, and a module for each contract type with its own
//! options.

mod acyclic;
mod encapsulation;
mod forbidden;
mod independence;
mod layers;
mod protected;

pub use acyclic::AcyclicConfig;
pub use encapsulation::EncapsulationConfig;
pub use forbidden::ForbiddenConfig;
pub use independence::IndependenceConfig;
pub use layers::{Layer, LayerMember, LayersConfig};
pub use protected::ProtectedConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zenith_graph::KindSet;

use crate::selector::{IgnoreImport, Selector};

/// The contract types, as written in `type`.
pub(crate) const CONTRACT_TYPES: [&str; 6] =
    ["forbidden", "independence", "layers", "protected", "acyclic", "encapsulation"];

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

    /// How it sees imports through barrels: its own `barrels`, or else
    /// `[analysis] barrels`. Always opaque for `encapsulation`, which is
    /// about importing through entry points, as barrels often are.
    pub barrels: Barrels,

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

    /// The modules that chains of imports may start or end at, but not pass
    /// through. Only the contract types that follow chains have them.
    pub fn boundaries(&self) -> &[Selector] {
        match self {
            Self::Forbidden(ForbiddenConfig { boundaries, .. })
            | Self::Independence(IndependenceConfig { boundaries, .. })
            | Self::Layers(LayersConfig { boundaries, .. }) => boundaries,
            Self::Protected(_) | Self::Acyclic(_) | Self::Encapsulation(_) => &[],
        }
    }
}

/// How a contract sees the imports of barrels, the files that only re-export
/// other modules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Barrels {
    /// An import of a barrel imports the barrel, which imports every module
    /// it re-exports.
    #[default]
    Opaque,

    /// An import of names from a barrel imports the modules that define
    /// them, as a bundler that leaves out unused modules loads them.
    Transparent,
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
