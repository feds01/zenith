//! A title that shows an `Option` field, which only its own notes can show.

use zenith_diagnostics::{Diagnostic, Span};

#[derive(Diagnostic)]
#[diag(test_disallowed)]
struct Disallowed {
    value: Option<String>,
    #[label]
    span: Span,
}

fn main() {}
