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
//! those that the project's [`Exemptions`] leave out of it: the statements
//! that `zenith-ignore` comments suppress, and the known violations that its
//! [`Baseline`] records. It returns a [`ContractReport`] of its
//! [`Violation`]s, which [`ContractReport::reports`] turns into reports with
//! excerpts of the imports that break it, and [`Violation::fix`] into a
//! [`Fix`] for each of their findings. The suppressions that had no effect
//! on any contract are each an [`UnusedSuppression`]. [`CheckJson`] is all of
//! it as the JSON output of `zenith check`.

mod baseline;
mod contract;
mod error;
mod findings;
#[cfg(test)]
mod packages;
mod reports;
mod rules;
mod schema;
mod selection;
mod suppressions;
#[cfg(test)]
mod testing;
mod view;

pub use baseline::{BASELINE_VERSION, Baseline, BaselineError};
pub use contract::{Contract, ContractReport, Exemptions};
pub use error::{ContractError, ContractErrorKind};
pub use findings::{Chain, CycleBreaker, CycleScope, Fix, FixAction, Link, Violation};
pub use schema::{CheckJson, JSON_VERSION};
pub use selection::{Selection, UnmatchedSelector};
pub use suppressions::{UnusedReason, UnusedSuppression};
