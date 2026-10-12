//! The logger behind zenith's `log!` statements, adapted from `bracketlint`.
//!
//! Unlike `bracketlint`'s, it writes every record to `stderr`: zenith prints
//! its reports on `stdout`, some of them for machines to read (e.g. `--format
//! json`), and log lines must never mix with them.

use std::io::{self, Write};

use log::{Level, Log, Metadata, Record};

use crate::highlight::{Colour, Modifier, highlight};

/// Writes log records to `stderr`, prefixed with their level, e.g.
/// `warning: ...`. Which levels are shown is set with [`log::set_max_level`].
#[derive(Debug, Default)]
pub struct ToolLogger;

impl ToolLogger {
    /// Create a logger.
    pub const fn new() -> Self {
        Self
    }
}

/// The prefix of a record of `level`, coloured when colour is on.
fn level_prefix(level: Level) -> String {
    match level {
        Level::Error => highlight(Colour::Red | Modifier::Bold, "error"),
        Level::Warn => highlight(Colour::Yellow | Modifier::Bold, "warning"),
        Level::Info => highlight(Colour::Blue | Modifier::Bold, "info"),
        Level::Debug => highlight(Colour::Blue | Modifier::Bold, "debug"),
        Level::Trace => highlight(Colour::Magenta | Modifier::Bold, "trace"),
    }
}

impl Log for ToolLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            // There is nowhere left to report a failure to write a log line.
            let _ = writeln!(
                io::stderr().lock(),
                "{}: {}",
                level_prefix(record.level()),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_are_prefixed_with_their_level() {
        assert_eq!(level_prefix(Level::Error), "error");
        assert_eq!(level_prefix(Level::Warn), "warning");
        assert_eq!(level_prefix(Level::Info), "info");
        assert_eq!(level_prefix(Level::Debug), "debug");
    }
}
