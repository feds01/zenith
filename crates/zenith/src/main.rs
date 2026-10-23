//! Entry point of zenith.

use std::{env, process::ExitCode};

use clap::Parser;
use zenith::{
    ExitStatus,
    cli::{Cli, expand_args},
    run,
};
use zenith_diagnostics::message;

/// mimalloc, which allocates and frees the many small values that parsing and
/// resolving make faster than the system allocator, on every platform.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn main() -> ExitCode {
    // Arguments are passed on as they are, without expanding globs on
    // Windows: selectors are globs that zenith matches itself.
    let args = match expand_args(env::args_os()) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{}", message!(cli_error, error = e));
            return ExitStatus::Error.into();
        }
    };

    let args = Cli::parse_from(args);

    match run(args) {
        Ok(status) => status.into(),
        Err(e) => {
            eprintln!("{}", message!(cli_error, error = format!("{e:#}")));
            ExitStatus::Error.into()
        }
    }
}
