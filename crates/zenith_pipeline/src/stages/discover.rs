//! The [`Discover`] stage.

use zenith_utils::path::relative_posix;
use zenith_workspace::{discover_packages, walk};

use crate::{PipelineError, Session, SourceFile, Stage, StageKind};

/// Finds the source files with `zenith_workspace`'s walker, and the workspace
/// packages among the `package.json` files it finds.
pub struct Discover;

impl Stage for Discover {
    fn kind(&self) -> StageKind {
        StageKind::Discover
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        let root = &session.root;
        let found = walk(root, &session.walk_settings());
        let workspace = discover_packages(root, &found.package_jsons);
        // The walk only finds files inside the root.
        let files = found
            .source_files
            .into_iter()
            .filter_map(|path| Some(SourceFile { name: relative_posix(root, &path)? }))
            .collect();

        session.files = files;
        session.workspace = workspace;
        Ok(())
    }
}
