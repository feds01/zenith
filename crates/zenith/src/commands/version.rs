//! `zenith version`.

use anyhow::Result;

use crate::ExitStatus;

/// Print the version of the `zenith` binary, with the commit it was built
/// from.
pub fn version() -> Result<ExitStatus> {
    println!("zenith {}", crate::version::version());
    Ok(ExitStatus::Success)
}
