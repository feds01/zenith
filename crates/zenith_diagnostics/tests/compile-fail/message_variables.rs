//! `message!` without a variable that the message shows, and with one that it
//! doesn't.

use zenith_diagnostics::message;

fn main() {
    message!(test_position);
    message!(test_empty, byte = 1);
}
