//! Library definition of the `zenith` crate.

pub mod cli;
mod commands;
mod crash;
mod project;
mod reporting;
mod sources;
pub(crate) mod version;

use std::{
    env, fmt,
    io::{IsTerminal, stdout},
    panic,
    process::ExitCode,
};

use anyhow::Result;
use cli::{ColourChoice, Command};
use crash::crash_handler;
use log::LevelFilter;
use zenith_utils::{highlight::set_colour_enabled, logging::ToolLogger};

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

/// The logger behind `log!` statements, which writes to `stderr`.
static LOGGER: ToolLogger = ToolLogger::new();

/// The stack size of rayon's worker threads. The parser recurses into nested
/// code, so deeply nested input, such as generated data, needs more than the
/// 2 MiB that threads get by default.
const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Handler function which will delegate functionality to the appropriate
/// command.
pub fn run(cli::Cli { command, global }: cli::Cli) -> Result<ExitStatus> {
    panic::set_hook(Box::new(crash_handler));

    // A logger set before, e.g. by an earlier run in the same process, stays.
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(match global.verbose {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        _ => LevelFilter::Debug,
    });
    set_colour_enabled(colour_enabled(global.color));
    // This fails only if the pool already exists, in which case it is kept.
    let _ = rayon::ThreadPoolBuilder::new().stack_size(WORKER_STACK_SIZE).build_global();

    let status = match command {
        Command::Check(args) => commands::check::check(&global, args),
        Command::Baseline(args) => commands::baseline::baseline(&global, args),
        Command::Graph(args) => commands::graph::graph(&global, args),
        Command::Why(args) => commands::why::why(&global, args),
        Command::Resolve(args) => commands::resolve::resolve(&global, args),
        Command::Init(args) => commands::init::init(&global, args),
        Command::Schema => commands::schema::schema(),
        Command::Version => commands::version::version(),
    };
    match status {
        Err(error) if error.is::<Reported>() => Ok(ExitStatus::Error),
        status => status,
    }
}

/// Whether to colour the output: by default, when `stdout` is a terminal and
/// `NO_COLOR` is unset or empty.
fn colour_enabled(choice: ColourChoice) -> bool {
    match choice {
        ColourChoice::Always => true,
        ColourChoice::Never => false,
        ColourChoice::Auto => {
            stdout().is_terminal() && env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
        }
    }
}
