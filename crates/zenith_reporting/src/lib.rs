//! The output of `zenith check`: the reports of what is wrong, then a summary
//! of every contract, in the `pretty`, `compact` or `github` format, or as a
//! SARIF log.
//!
//! The crate knows nothing of what is checked. The crates that check things
//! turn what they find into [`Report`](zenith_diagnostics::Report)s and
//! [`SummaryEntry`]s, and an [`Output`] shows them: the reports and a summary
//! of every contract (`pretty`), a line per finding (`compact`), or a GitHub
//! Actions workflow command per finding (`github`). A [`Sarif`] log shows the
//! reports as results of the [`SarifRule`]s they break, for code scanning.

mod output;
mod sarif;
mod summary;

pub use output::{Format, Output};
pub use sarif::{Sarif, SarifRule};
pub use summary::{Count, Outcome, Summary, SummaryEntry};
