//! `zenith baseline`.

use anyhow::Result;

use crate::{
    ExitStatus,
    cli::{BaselineCommand, GlobalArgs},
};

/// Record the current violations in the baseline file.
#[expect(clippy::unimplemented, reason = "not implemented yet")]
pub fn baseline(_global: &GlobalArgs, _args: BaselineCommand) -> Result<ExitStatus> {
    unimplemented!()
}
