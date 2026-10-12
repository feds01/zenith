//! The output of `zenith check`: the reports of what is wrong, then a summary
//! of every contract, in the `pretty`, `compact`, `github` or `json` format.
//!
//! The crate knows nothing of what is checked. The crates that check things
//! turn what they find into [`Report`](zenith_diagnostics::Report)s and
//! [`SummaryEntry`]s, and an [`Output`] shows them: the reports and a summary
//! of every contract (`pretty`), a line per finding (`compact`), or a GitHub
//! Actions workflow command per finding (`github`). [`Json`] shows the
//! summary, and what they found as JSON, for tools to read (`json`).

mod json;
mod output;
mod summary;

pub use json::{JSON_VERSION, Json, milliseconds};
pub use output::{Format, Output};
pub use summary::{Count, Outcome, Summary, SummaryEntry};
