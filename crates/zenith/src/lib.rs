//! Library definition of the `zenith` crate.

pub mod cli;
mod commands;
mod crash;
mod logger;
mod project;
mod reporting;
pub(crate) mod version;

use std::{fmt, panic, process::ExitCode};

use anyhow::Result;
use cli::Command;
use crash::crash_handler;
use zenith_utils::{highlight::set_colour_enabled, pool::setup_global_threads};

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

/// An error that has already been shown, e.g. as reports of an invalid
/// configuration file, so all that is left is to exit with
/// [`ExitStatus::Error`].
#[derive(Debug)]
pub(crate) struct Reported;

impl fmt::Display for Reported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the errors have been reported")
    }
}

impl std::error::Error for Reported {}

/// Handler function which will delegate functionality to the appropriate
/// command.
pub fn run(cli::Cli { command, global }: cli::Cli) -> Result<ExitStatus> {
    panic::set_hook(Box::new(crash_handler));
    logger::setup_logging(global.log_level());
    set_colour_enabled(global.color.should_colour());
    setup_global_threads();

    let status = match command {
        Command::Check(args) => commands::check::check(&global, args),
        Command::Baseline(args) => commands::baseline::baseline(&global, args),
        Command::Graph(args) => commands::graph::graph(&global, args),
        Command::Why(args) => commands::why::why(&global, args),
        Command::Resolve(args) => commands::resolve::resolve(&global, args),
        Command::Init(args) => commands::init::init(&global, args),
        Command::Suggest => commands::suggest::suggest(&global),
        Command::Schema => commands::schema::schema(),
        Command::Version => commands::version::version(),
    };
    match status {
        Err(error) if error.is::<Reported>() => Ok(ExitStatus::Error),
        status => status,
    }
}
