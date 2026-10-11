//! The `zenith.toml` configuration file of a zenith project.
//!
//! This crate checks the syntax of the values that contracts are written
//! with: [`Selector`]s, which pick the modules a contract is about, the
//! [`Layer`]s of a `layers` contract, and the [`IgnoreImport`] entries that
//! pair two selectors.

mod layer;
mod selector;

pub use layer::{Layer, LayerError, LayerMember};
pub use selector::{
    BuiltinScheme, IgnoreImport, IgnoreImportError, Selector, SelectorError, SelectorTarget,
};
