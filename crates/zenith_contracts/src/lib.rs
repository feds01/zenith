//! Architecture contracts: compiling them against a project, and checking
//! them.
//!
//! A contract is about the modules that its selectors pick. Each selector
//! resolves, against the import graph and workspace packages of the project,
//! into a [`Selection`] of [`Unit`](zenith_graph::Unit)s: labelled sets of
//! the nodes that take part in imports. A path or `pkg:` selector that
//! matches nothing is an [`UnmatchedSelector`].
//!
//! [`Contract::compile`] resolves the selectors of a contract, and fails with
//! [`ContractError`]s that point at the selectors in the configuration file.
//! [`Contract::check`] then checks it against its own view of the graph,
//! without the imports that the contract excludes by kind or ignores, and
//! returns a [`ContractReport`] of its [`Violation`]s.

mod contract;
mod error;
mod findings;
mod rules;
mod selection;
#[cfg(test)]
mod testing;
mod view;

pub use contract::{Contract, ContractReport};
pub use error::{ContractError, ContractErrorKind};
pub use findings::{Chain, Link, Violation};
pub use selection::{Selection, UnmatchedSelector};
