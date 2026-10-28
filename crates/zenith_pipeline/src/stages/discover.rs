//! The [`Discover`] stage, and the walk that the globs of configuration
//! files are matched against, which finds files as it does.

use std::path::{Path, PathBuf};

use zenith_config::{FilesConfig, ProjectWalk};
use zenith_lang::Workspace;
use zenith_utils::{fs::FileSystem, glob::Glob, path::relative_posix};
use zenith_workspace::{Git, WalkSettings, walk};

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
        let mut found = walk(&*session.file_system, root, &session.walk_settings());
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

/// The walk of a project that the globs of its configuration files are
/// matched against: `zenith_workspace`'s walker, which skips what zenith
/// never scans, as [`Discover`] does.
pub struct ConfigWalk;

impl ProjectWalk for ConfigWalk {
    fn files(
        &self,
        file_system: &dyn FileSystem,
        root: &Path,
        files: &FilesConfig,
    ) -> Vec<PathBuf> {
        let settings = WalkSettings {
            include: vec![Glob::new("**").expect("`**` is a valid glob")],
            exclude: files.exclude.clone(),
            respect_gitignore: files.respect_gitignore,
            source_extensions: Vec::new(),
            source_file_names: Vec::new(),
            manifests: Vec::new(),
        };
        walk(file_system, root, &settings).source_files
    }
}
