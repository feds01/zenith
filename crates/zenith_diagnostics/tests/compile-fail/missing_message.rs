//! A diagnostic, or `message!`, that names a message the catalogue lacks.

use zenith_diagnostics::{Diagnostic, message};

#[derive(Diagnostic)]
#[diag(test_missing)]
struct Missing;

fn main() {
    message!(test_missing);
}
