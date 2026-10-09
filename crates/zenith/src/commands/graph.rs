//! `zenith graph`.

use anyhow::Result;

use crate::{
    ExitStatus,
    cli::{GlobalArgs, GraphCommand},
};

/// Show the module tree under a selector, and the dependencies between its
/// children.
#[expect(clippy::unimplemented, reason = "not implemented yet")]
pub fn graph(_global: &GlobalArgs, _args: GraphCommand) -> Result<ExitStatus> {
    unimplemented!()
}
