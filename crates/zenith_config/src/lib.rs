//! The `zenith.toml` configuration file: finding it, parsing it, and
//! validating it into a [`Config`].
//!
//! [`Config::discover`] finds the nearest configuration file, and
//! [`Config::load`] reads it and parses it with [`Config::parse`]. Parsing
//! rejects unknown keys everywhere, with "did you mean" suggestions, and checks
//! the syntax of every selector, glob, layer and ignored import, so that a
//! [`Config`] only holds valid values. Every [`ConfigError`] has the byte
//! range of the offending key or value, when the TOML parser knows it, and
//! [`ConfigError::report`] shows it in context.
//!
//! [`json_schema`] describes the file for editors, generated from the same
//! types that it is deserialised into.

mod config;
mod contracts;
mod error;
mod parse;
mod schema;
mod selector;

pub use config::{
    AnalysisConfig, BaselineConfig, CONFIG_FILE_NAME, Config, FilesConfig, ResolveConfig,
    StaleLevel, Tsconfig,
};
pub use contracts::{
    AcyclicConfig, AlertLevel, ContractConfig, ContractKind, EncapsulationConfig, ForbiddenConfig,
    IndependenceConfig, Layer, LayerMember, LayersConfig, ProtectedConfig, Severity,
};
pub use error::{
    ConfigError, ConfigErrorKind, IgnoreImportError, LayerError, LoadError, SelectorError,
};
pub use schema::json_schema;
pub use selector::{BuiltinScheme, IgnoreImport, Selector, SelectorTarget};
