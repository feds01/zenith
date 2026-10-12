//! The errors and warnings that zenith reports about its input, such as an
//! invalid configuration file, and their rendering.
//!
//! A pass over some input collects what it finds in a [`DiagnosticStore`],
//! as typed errors and warnings that its callers and tests can inspect. To
//! show them, each is turned into a [`Report`]: a title, excerpts of the
//! sources that it is about with the offending text labelled, and notes. A
//! [`Reporter`] renders reports with `annotate-snippets`, in full or a line
//! each, reading the excerpts from a [`SourceMap`].
//!
//! The [`SourceMap`] is where zenith finds what is in a source and where:
//! it reads a project's files when they are first needed, names them as
//! reports show them, and finds the [`Location`] that a [`Span`] starts at.

mod report;
mod reporter;
mod source;
mod store;

pub use report::{Report, ReportCodeBlock, ReportElement, ReportKind, ReportNote, ReportNoteKind};
pub use reporter::Reporter;
pub use source::{Location, Position, Source, SourceId, SourceMap, Span};
pub use store::DiagnosticStore;
