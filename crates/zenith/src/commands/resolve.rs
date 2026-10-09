//! `zenith resolve`.

use anyhow::Result;

use crate::{
    ExitStatus,
    cli::{GlobalArgs, ResolveCommand},
};

/// Show how one import specifier resolves from a file, and why.
#[expect(clippy::unimplemented, reason = "not implemented yet")]
pub fn resolve(_global: &GlobalArgs, _args: ResolveCommand) -> Result<ExitStatus> {
    unimplemented!()
}
