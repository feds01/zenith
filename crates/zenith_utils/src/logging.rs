//! This defines a simple logger with a style which should be used across
//! zenith to log and print messages.
//!
//! Unlike `bracketlint`, every record goes to the error stream: zenith prints
//! machine readable reports on `stdout` (e.g. `--format json`), and log
//! lines must never interleave with them.

use std::{io::Write, sync::OnceLock};

use log::{Level, Log, Metadata, Record};

use crate::{
    highlight::{Colour, Modifier, highlight},
    stream::CompilerOutputStream,
    stream_writeln,
};

/// The logger that is used for `log!` statements.
#[derive(Default, Debug)]
pub struct ToolLogger {
    /// The stream that the logger writes to, defaults to `stderr`.
    pub error_stream: OnceLock<CompilerOutputStream>,
}

impl ToolLogger {
    /// Create a new logger.
    pub const fn new() -> Self {
        Self { error_stream: OnceLock::new() }
    }
}

impl Log for ToolLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        // Custom colour formatting for the log level
        let level_prefix = match record.level() {
            Level::Error => highlight(Colour::Red | Modifier::Bold, "error"),
            Level::Warn => highlight(Colour::Yellow | Modifier::Bold, "warning"),
            Level::Info => highlight(Colour::Blue | Modifier::Bold, "info"),
            Level::Debug => highlight(Colour::Blue | Modifier::Bold, "debug"),
            Level::Trace => highlight(Colour::Magenta | Modifier::Bold, "trace"),
        };

        let mut out = self.error_stream.get().cloned().unwrap_or_else(CompilerOutputStream::stderr);
        stream_writeln!(out, "{level_prefix}: {}", record.args());
    }

    fn flush(&self) {}
}
