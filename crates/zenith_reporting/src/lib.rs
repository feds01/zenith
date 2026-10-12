//! The output of `zenith check`: the reports of what is wrong, then a summary
//! of every contract, in the `pretty` or `compact` format.
//!
//! The crate knows nothing of what is checked. The crates that check things
//! turn what they find into [`Report`](zenith_diagnostics::Report)s and
//! [`SummaryEntry`]s, and an [`Output`] shows them.

mod output;
mod summary;

pub use output::{Format, Output};
pub use summary::{Outcome, Summary, SummaryEntry};
