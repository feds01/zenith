//! `zenith version`.

use anyhow::Result;

use crate::ExitStatus;

pub fn version() -> Result<ExitStatus> {
    println!("zenith {}", crate::version::version());
    Ok(ExitStatus::Success)
}
