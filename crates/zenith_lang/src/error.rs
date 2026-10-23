//! Syntax errors in source files, and their reports.

use std::{borrow::Cow, fmt};

use zenith_diagnostics::Diagnostic;

/// A syntax error in a source file. It is located by the code that the
/// parser points at, whose range is kept beside it, in a `Located`: to mark
/// it, make the report with `report_in` the file's source, then `add_span`
/// the range.
#[derive(Debug, Clone, PartialEq, Eq, Diagnostic)]
#[diag(lang_syntax_error)]
pub struct SyntaxError {
    /// What is wrong, as the parser describes it, e.g. ``Expected `,` but
    /// found `;` ``.
    pub message: Cow<'static, str>,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SyntaxError {}
