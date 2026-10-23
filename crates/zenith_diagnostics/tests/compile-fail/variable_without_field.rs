//! A message that shows a variable which the diagnostic has no field for.

use zenith_diagnostics::Diagnostic;

#[derive(Diagnostic)]
#[diag(test_position)]
struct Position {
    offset: usize,
}

fn main() {}
