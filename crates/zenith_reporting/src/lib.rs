//! Showing the results of checking contracts.
//!
//! A [`ViolationReporter`] turns what checking a contract finds into
//! [`Report`](zenith_diagnostics::Report)s, with excerpts of the imports
//! that break it, read from the project's [`Sources`]. [`CheckResults`] writes
//! the output of `zenith check`: those reports and a summary of every
//! contract (`pretty`), or a line per finding (`compact`).

mod check;
mod sources;
mod violations;

pub use check::CheckResults;
pub use sources::Sources;
pub use violations::ViolationReporter;
