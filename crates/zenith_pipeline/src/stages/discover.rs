//! The [`Discover`] stage.

use std::path::PathBuf;

use zenith_lang::Workspace;
use zenith_utils::path::relative_posix;
use zenith_workspace::{Git, walk};

use crate::{PipelineError, Session, SourceFile, Stage, StageKind};

/// Finds the source files with `zenith_workspace`'s walker, each with its
/// language, and each language's packages among the manifest files it finds.
/// With [`Session::tracked_only`], only the files that git tracks count.
pub struct Discover;

impl Stage for Discover {
    fn kind(&self) -> StageKind {
        StageKind::Discover
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        // 1. Walk the project.
        let root = &session.root;
        let mut found = walk(root, &session.walk_settings());
        if session.tracked_only {
            found.retain_tracked(&Git::new(root).tracked_files()?);
        }

        // 2. Name each source file, and find its language. The walk only finds files
        //    inside the root, of one of the languages.
        let languages = &session.config.languages;
        let files = found
            .source_files
            .into_iter()
            .filter_map(|path| {
                let name = relative_posix(root, &path)?;
                let language = u8::try_from(languages.language_of(&name)?).ok()?;
                Some(SourceFile { name, language })
            })
            .collect();

        // 3. Each language finds its packages among its manifests.
        let mut workspace = Workspace::default();
        for backend in session.backends.iter_mut() {
            let names = backend.manifests();
            let manifests: Vec<PathBuf> = (found.manifests.iter())
                .filter(|path| {
                    path.file_name().is_some_and(|name| names.iter().any(|known| name == *known))
                })
                .cloned()
                .collect();
            workspace.merge(backend.discover(&manifests));
        }

        session.files = files;
        session.workspace = workspace;
        Ok(())
    }
}
