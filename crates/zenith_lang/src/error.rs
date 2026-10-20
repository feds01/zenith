//! Syntax errors in source files, and their reports.

use std::{borrow::Cow, fmt};

use zenith_diagnostics::{self as diagnostics, Report};

/// A syntax error in a source file. It is located by the code that the
/// parser points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// What is wrong, as the parser describes it, e.g. ``Expected `,` but
    /// found `;` ``.
    pub message: Cow<'static, str>,
}

impl SyntaxError {
    /// The report of the error, which points at `span`, where the error is.
    pub fn report(&self, span: diagnostics::Span) -> Report {
        let mut report = Report::error(self.message.as_ref());
        report.add_span(span);
        report
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SyntaxError {}
