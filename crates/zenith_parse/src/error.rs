//! Syntax errors in source files, and their reports.

use std::{borrow::Cow, fmt};

use oxc_span::{LabeledSpan, Span};
use zenith_diagnostics::{self as diagnostics, Report};
use zenith_utils::location::{ByteRange, Located};

/// A syntax error in a source file. It is located by the code that the
/// parser points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// What is wrong, as the parser describes it, e.g. ``Expected `,` but
    /// found `;` ``.
    pub message: Cow<'static, str>,
}

impl SyntaxError {
    /// An error with the parser's `message`, at the code its primary label
    /// points at, or its first label if none is primary.
    pub(crate) fn located(message: Cow<'static, str>, labels: &[LabeledSpan]) -> Located<Self> {
        let label = labels.iter().find(|label| label.primary()).or_else(|| labels.first());
        let span = label.map_or(Span::default(), LabeledSpan::span);
        Located::new(ByteRange::new(span.start, span.end), Self { message })
    }

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
