//! The `zenith.toml` configuration file: finding it, parsing it, and
//! validating it into a [`Config`].
//!
//! [`Config::discover`] finds the project's configuration file, and
//! [`Config::load`] reads it and parses it with [`Config::parse`], with the
//! files it is split over: those it includes, which are read as if their
//! contracts were written in it, and those of its members, directories with
//! a `zenith.toml` of their own, whose paths are relative to the member and
//! whose ids start with it. Parsing
//! rejects unknown keys everywhere, with "did you mean" suggestions, and checks
//! the syntax of every selector, glob, layer and ignored import, so that a
//! [`Config`] only holds valid values. Every [`ConfigError`] has the byte
//! range of the offending key or value, when the TOML parser knows it, and
//! [`ConfigError::report`] shows it in context.
//!
//! Each language has a table of its own in `[languages]`. This crate knows no
//! language: each is a [`Language`], defined in its own crate, which says what
//! it owns: its default files, the extensions that path selectors may leave
//! off, its selector schemes, and how to parse its table. The languages that a
//! file may enable are given to [`Config::parse`] as [`Languages`].
//!
//! [`json_schema`] describes the file for editors, generated from the same
//! types that it is deserialised into.

mod config;
mod contracts;
mod error;
mod language;
mod parse;
mod parts;
mod schema;
mod selector;
#[cfg(test)]
mod testing;

pub use config::{
    AnalysisConfig, BaselineConfig, CacheConfig, Config, ConfigFile, ConfigSources, FilesConfig,
    LanguagesConfig, ProjectWalk, Scanned, StaleLevel,
};
pub use contracts::{
    AcyclicConfig, AlertLevel, ContractConfig, ContractKind, EncapsulationConfig, ForbiddenConfig,
    IndependenceConfig, Layer, LayerMember, LayersConfig, LimitsConfig, ProtectedConfig, Reexports,
    Severity,
};
pub use error::{
    Alternatives, ConfigError, ConfigErrorKind, IgnoreImportError, InContract, InvalidFile,
    LayerError, LoadError, PatternError, SelectorError,
};
pub use language::{Language, LanguageOptions, Languages, NEUTRAL_SCHEMES};
pub use parse::LanguageTable;
pub use parts::CONFIG_FILE_NAME;
pub use schema::{Located, json_schema};
pub use selector::{IgnoreImport, Selector, SelectorTarget};
