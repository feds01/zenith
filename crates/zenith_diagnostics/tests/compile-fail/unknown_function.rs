//! A message that calls a function the catalogue doesn't register.

use zenith_diagnostics::message;

fn main() {
    message!(test_unknown_function, name = "a");
}
