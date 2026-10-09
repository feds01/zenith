//! Library definition of the `zenith` crate.

pub mod cli;
mod commands;
mod crash;
pub(crate) mod version;

use std::{panic, process::ExitCode};

use anyhow::Result;
use cli::Command;
use crash::crash_handler;

/// How a command finished, which determines the process exit code.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    /// The command succeeded: every contract was kept.
    Success,

    /// The command ran, but contracts were broken.
    Failure,

    /// The command could not run: bad configuration, usage or an internal
    /// error.
    Error,
}

impl From<ExitStatus> for ExitCode {
    fn from(status: ExitStatus) -> Self {
        match status {
            ExitStatus::Success => ExitCode::from(0),
            ExitStatus::Failure => ExitCode::from(1),
            ExitStatus::Error => ExitCode::from(2),
        }
    }
}

/// Handler function which will delegate functionality to the appropriate
/// command.
pub fn run(cli::Cli { command, global }: cli::Cli) -> Result<ExitStatus> {
    panic::set_hook(Box::new(crash_handler));

    match command {
        Command::Check(args) => commands::check::check(&global, args),
        Command::Baseline(args) => commands::baseline::baseline(&global, args),
        Command::Graph(args) => commands::graph::graph(&global, args),
        Command::Why(args) => commands::why::why(&global, args),
        Command::Resolve(args) => commands::resolve::resolve(&global, args),
        Command::Init(args) => commands::init::init(&global, args),
        Command::Schema => commands::schema::schema(),
        Command::Version => commands::version::version(),
    }
}
