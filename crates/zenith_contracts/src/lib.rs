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
//! on any contract are each an [`UnusedSuppression`]. The [`Summary`] has a
//! line for each checked contract, which an [`Output`] shows after the
//! reports in a [`Format`]. [`ContractsJson`] is all of it as the JSON output
//! of `zenith check` shows it, and [`ContractReport::sarif`] as its SARIF
//! output.
//!
//! [`Suggestion::find`] proposes contracts that the code already keeps,
//! checking each one before it is suggested.

mod baseline;
mod contract;
mod error;
mod findings;
mod output;
#[cfg(test)]
mod packages;
mod reports;
mod rules;
mod sarif;
mod schema;
mod selection;
mod suggest;
mod summary;
mod suppressions;
#[cfg(test)]
mod testing;
mod view;

pub use baseline::{BASELINE_VERSION, Baseline, BaselineError};
pub use contract::{Contract, ContractReport, Exemptions};
pub use error::{ContractError, ContractErrorKind};
pub use findings::{Chain, CycleBreaker, CycleScope, Fix, FixAction, Limit, Link, Violation};
pub use output::{Format, Output};
pub use schema::ContractsJson;
pub use selection::{Selection, UnmatchedSelector};
pub use suggest::Suggestion;
pub use summary::{Count, Outcome, Summary, SummaryEntry};
pub use suppressions::{UnusedReason, UnusedSuppression};
