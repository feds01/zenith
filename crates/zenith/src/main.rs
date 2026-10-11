//! Entry point of zenith.

use std::{env, process::ExitCode};

use clap::Parser;
use zenith::{
    ExitStatus,
    cli::{Cli, expand_args},
    run,
};

pub fn main() -> ExitCode {
    // Arguments are passed on as they are, without expanding globs on
    // Windows: selectors are globs that zenith matches itself.
    let args = match expand_args(env::args_os()) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitStatus::Error.into();
        }
    };

    let args = Cli::parse_from(args);

    match run(args) {
        Ok(status) => status.into(),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitStatus::Error.into()
        }
    }
}
