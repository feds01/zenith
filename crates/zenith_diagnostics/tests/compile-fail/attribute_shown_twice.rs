//! A diagnostic that shows an attribute of its message twice.

use zenith_diagnostics::Diagnostic;

#[derive(Diagnostic)]
#[diag(test_mismatch)]
#[help]
struct Mismatch {
    expected: char,
    found: char,
    #[help]
    quoted: bool,
}

fn main() {}
