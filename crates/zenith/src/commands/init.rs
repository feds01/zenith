//! `zenith init`.

use anyhow::Result;

use crate::{
    ExitStatus,
    cli::{GlobalArgs, InitCommand},
};

/// Write a starter `zenith.toml` from the discovered packages.
#[expect(clippy::unimplemented, reason = "not implemented yet")]
pub fn init(_global: &GlobalArgs, _args: InitCommand) -> Result<ExitStatus> {
    unimplemented!()
}
