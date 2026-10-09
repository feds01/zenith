//! `zenith why`.

use anyhow::Result;

use crate::{
    ExitStatus,
    cli::{GlobalArgs, WhyCommand},
};

/// Show the shortest import chains from one selector to another.
#[expect(clippy::unimplemented, reason = "not implemented yet")]
pub fn why(_global: &GlobalArgs, _args: WhyCommand) -> Result<ExitStatus> {
    unimplemented!()
}
