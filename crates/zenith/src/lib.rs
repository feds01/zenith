//! Library definition of the `zenith` crate.

pub mod cli;
mod commands;
mod crash;
mod project;
pub(crate) mod version;

use std::{
    io::{IsTerminal, stdout},
    panic,
    process::ExitCode,
};

use anyhow::Result;
use cli::{ColourChoice, Command};
use crash::crash_handler;
use zenith_utils::{
    highlight::set_colour_enabled, logging::ToolLogger, stream::CompilerOutputStream,
};

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

pub static LOGGER: ToolLogger = ToolLogger::new();

/// Handler function which will delegate functionality to the appropriate
/// command.
pub fn run(cli::Cli { command, global }: cli::Cli) -> Result<ExitStatus> {
    // Initial grunt work, panic handler and logger setup...
    panic::set_hook(Box::new(crash_handler));

    log::set_logger(&LOGGER).unwrap_or_else(|_| panic!("couldn't initiate logger"));
    LOGGER.error_stream.set(CompilerOutputStream::stderr()).unwrap();
    log::set_max_level(match global.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        _ => log::LevelFilter::Debug,
    });

    let machine_output = matches!(
        &command,
        Command::Check(cli::CheckCommand { format: cli::Format::Json | cli::Format::Github, .. })
            | Command::Schema
    );
    set_colour_enabled(match global.color {
        ColourChoice::Always => true,
        ColourChoice::Never => false,
        ColourChoice::Auto => {
            !machine_output
                && stdout().is_terminal()
                && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
        }
    });

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
