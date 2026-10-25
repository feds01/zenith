//! The language server of `zenith lsp`, which publishes what zenith finds to
//! an editor as diagnostics, over the Language Server Protocol.
//!
//! A [`Server`] starts once the editor initialises it. It then checks the
//! project, publishes what the check found, and does it again whenever the
//! editor saves a file, until the editor shuts it down. It tells each check
//! the [`Changes`] since the last: the files that the editor saved, and those
//! that it saw change, if it reports the changes to files. It knows nothing of
//! what is checked: a check gives it what it [`Found`], as
//! [`Report`](zenith_diagnostics::Report)s, and each report with an excerpt
//! is a diagnostic on the file of its first excerpt, with its other excerpts
//! as related information. The reports of code that has no effect are hints,
//! which editors fade. A file that stops having diagnostics is cleared.
//!
//! A check that fails keeps the diagnostics of the last one that ran. Its
//! [`Failure`] is shown on the files that its reports point into, such as
//! the configuration file, until a check runs again, or else as a message.
//!
//! Positions count UTF-8 bytes if the editor supports it, as zenith's
//! offsets do, or else characters, and otherwise UTF-16 code units, which
//! the protocol requires every editor to support.

mod diagnostics;
mod error;
mod server;

pub use error::ServerError;
pub use server::{Changes, Failure, Found, Server};
