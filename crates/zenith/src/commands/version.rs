//! `zenith version`.

use anyhow::Result;
use zenith_diagnostics::message;

use crate::ExitStatus;

/// Print the version of the `zenith` binary, with the commit it was built
/// from.
pub fn version() -> Result<ExitStatus> {
    println!("{}", message!(cli_version, version = crate::version::version()));
    Ok(ExitStatus::Success)
}
