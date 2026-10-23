//! A title from a field as well as from a message, from neither, from a
//! field that names a message or is an `Option`, and notes on a diagnostic
//! whose title comes from a field, which has no message for them.

use zenith_diagnostics::Diagnostic;

#[derive(Diagnostic)]
#[diag(test_empty)]
struct Empty;

#[derive(Diagnostic)]
#[diag(test_empty)]
struct TwoTitles {
    #[diag]
    error: Empty,
}

#[derive(Diagnostic)]
struct NoTitle {
    error: Empty,
}

#[derive(Diagnostic)]
struct NamedOnAField {
    #[diag(test_empty)]
    error: Empty,
}

#[derive(Diagnostic)]
struct OptionalTitle {
    #[diag]
    error: Option<Empty>,
}

#[derive(Diagnostic)]
#[note]
struct NoteWithoutMessage {
    #[diag]
    error: Empty,
}

fn main() {}
