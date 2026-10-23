//! A diagnostic that doesn't show an attribute of its message.

use zenith_diagnostics::Diagnostic;

#[derive(Diagnostic)]
#[diag(test_mismatch)]
struct Mismatch {
    expected: char,
    found: char,
}

fn main() {}
