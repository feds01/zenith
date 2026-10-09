//! `zenith check`.

use anyhow::Result;

use crate::{
    ExitStatus,
    cli::{CheckCommand, GlobalArgs},
};

/// Check the architecture contracts.
#[expect(clippy::unimplemented, reason = "not implemented yet")]
pub fn check(_global: &GlobalArgs, _args: CheckCommand) -> Result<ExitStatus> {
    unimplemented!()
}
