//! Why the server stopped talking to the editor.

use std::{fmt, io};

use lsp_server::ProtocolError;
use zenith_diagnostics::message;

/// Why the server stopped talking to the editor, before the editor told it
/// to exit.
#[derive(Debug)]
pub enum ServerError {
    /// The editor broke the protocol, such as by not initialising the server
    /// first.
    Protocol(ProtocolError),

    /// A message to or from the editor isn't what the protocol says.
    Message(serde_json::Error),

    /// The editor went away, so nothing more can be sent to it.
    Disconnected,

    /// stdin or stdout failed.
    Io(io::Error),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Protocol(error) => message!(lsp_error_protocol, error = error.to_string()),
            Self::Message(error) => message!(lsp_error_message, error = error.to_string()),
            Self::Disconnected => message!(lsp_error_disconnected),
            Self::Io(error) => message!(lsp_error_io, error = error.to_string()),
        };
        f.write_str(&text)
    }
}

/// The message includes the cause, so the error has no `source`: reporters
/// that show the chain of causes would show it twice.
impl std::error::Error for ServerError {}

impl From<ProtocolError> for ServerError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

impl From<serde_json::Error> for ServerError {
    fn from(error: serde_json::Error) -> Self {
        Self::Message(error)
    }
}

impl From<io::Error> for ServerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
