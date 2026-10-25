//! The errors and warnings that zenith reports about its input, such as an
//! invalid configuration file, and their rendering.
//!
//! A pass over some input collects what it finds in a [`DiagnosticStore`],
//! as typed errors and warnings that its callers and tests can inspect. To
//! show them, each is turned into a [`Report`]: a title, excerpts of the
//! sources that it is about with the offending text labelled, and notes. An
//! error makes its report as a [`Diagnostic`], usually derived, from the
//! messages of the catalogue: what zenith says to its users is kept there,
//! in Fluent files under `messages/`, rather than in its code, which names
//! each message by id, with [`message!`] or the derive. A [`Reporter`] renders
//! reports with `annotate-snippets`, in full or a line each, reading the
//! excerpts from a [`SourceMap`], and a [`Sarif`] log shows them as the
//! results of the [`SarifRule`]s they break, for code scanning.
//!
//! The [`SourceMap`] is where zenith finds what is in a source and where:
//! it reads a project's files when they are first needed, names them as
//! reports show them, and finds the [`Location`] that a [`Span`] starts at.

mod catalogue;
mod diagnostic;
mod report;
mod reporter;
mod sarif;
mod source;
mod store;

pub use diagnostic::{Diagnostic, Subdiagnostic};
pub use report::{
    Report, ReportCodeBlock, ReportElement, ReportKind, ReportNote, ReportNoteKind, ReportSpan,
};
pub use reporter::Reporter;
pub use sarif::{Sarif, SarifRule};
pub use source::{Columns, Location, Position, Source, SourceId, SourceMap, Span};
pub use store::DiagnosticStore;
pub use zenith_macros::{Diagnostic, message};

/// What the macros of zenith_macros expand to. Code formats messages with
/// [`message!`] or the derive of [`Diagnostic`], which check the ids and
/// variables that it gives when it compiles; these don't.
#[doc(hidden)]
pub mod __private {
    pub use crate::catalogue::{MessageArgs, message};
}
