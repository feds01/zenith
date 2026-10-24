//! `zenith suggest`.

use anyhow::Result;
use log::warn;
use zenith_contracts::Suggestion;
use zenith_diagnostics::message;
use zenith_pipeline::{LANGUAGES, StageKind};

use crate::{ExitStatus, cli::GlobalArgs, project};

/// Print, as TOML, the contracts that the code already keeps, each with a
/// comment that says why it is suggested.
pub fn suggest(global: &GlobalArgs) -> Result<ExitStatus> {
    // 1. Open the project, and build its graph, through re-exports if the suggested
    //    contracts see through them.
    let mut session = project::open(global)?;
    if Suggestion::needs_graph_through_reexports(&session.config) {
        session.build_graph_through_reexports();
    }
    project::run_until(&mut session, StageKind::Graph)?;
    project::warn_about_unresolved(&session);

    // 2. Find the contracts that its code keeps, checked as the configuration's
    //    contracts would be.
    let suggestions =
        Suggestion::find(session.graphs(), &session.workspace, &session.config, LANGUAGES);

    // 3. Print them, a blank line apart.
    if suggestions.is_empty() {
        warn!("{}", message!(cli_suggest_none));
    }
    let tables: Vec<_> = suggestions.iter().map(Suggestion::to_string).collect();
    print!("{}", tables.join("\n"));
    Ok(ExitStatus::Success)
}
