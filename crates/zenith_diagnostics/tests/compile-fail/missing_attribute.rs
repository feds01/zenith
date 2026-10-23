//! A note that names an attribute the message lacks.

use zenith_diagnostics::Diagnostic;

#[derive(Diagnostic)]
#[diag(test_empty)]
#[help]
struct Empty;

fn main() {}
