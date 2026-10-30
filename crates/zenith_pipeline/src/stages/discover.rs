//! The [`Discover`] stage, and the walk that the globs of configuration
//! files are matched against, which finds files as it does.

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use log::info;
use zenith_cache::{CacheSettings, Cached, Decoder, Encoder, Snapshot};
use zenith_config::{FilesConfig, ProjectWalk};
use zenith_diagnostics::message;
use zenith_lang::Workspace;
use zenith_utils::{
    fs::{FileSystem, adjust_canonicalisation},
    glob::Glob,
    path::{absolute_path, relative_posix},
};
use zenith_workspace::{
    Git, WalkResult, WalkSettings, walk, walk_dependencies_outside, walk_recording,
};

use crate::{PipelineError, Session, SourceFile, Stage, StageKind};

/// Finds the source files with `zenith_workspace`'s walker, each with its
/// language, and each language's packages among the manifest files it finds.
/// With [`Session::tracked_only`], only the files that git tracks count.
///
/// With [`Session::cache`] set, a walk of the disk is kept in the cache, and
/// taken from it instead of walking again while nothing that the walk read
/// has changed: see [`Snapshot`].
pub struct Discover;

impl Stage for Discover {
    fn kind(&self) -> StageKind {
        StageKind::Discover
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        // 1. Walk the project, or take what the last walk found.
        let root = &session.root;
        let settings = session.walk_settings();
        let kept = match &session.cache {
            Some(cache) if session.file_system.on_disk() => Some(walk_kept(cache, root, &settings)),
            _ => None,
        };
        session.stats.cached_walk = kept.as_ref().is_some_and(|(_, cached)| *cached);
        let mut found = match kept {
            Some((found, _)) => found,
            None => walk(&*session.file_system, root, &settings),
        };
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

/// The walk of the disk at `root` with `settings`: what the last walk found,
/// kept in the cache of `cache`, if nothing that it read has changed since,
/// or else what walking again finds, which is then kept. Also whether it came
/// from the cache.
fn walk_kept(cache: &CacheSettings, root: &Path, settings: &WalkSettings) -> (WalkResult, bool) {
    // 1. Take what the last walk found, if nothing it read has changed since.
    let snapshot = Snapshot::<Walked>::new(cache.clone(), (root, settings.inputs()));
    if let Some(walked) = snapshot.load() {
        return (walked.result(root), true);
    }

    // 2. Otherwise walk, having seen what the walk depends on outside the
    //    directories that it reads before it starts, so that a change to it while
    //    it walks isn't kept.
    let started = SystemTime::now();
    let outside = walk_dependencies_outside(root, settings);
    let before = snapshot.observe(&outside.contents, &outside.kinds);
    let (found, inside) = walk_recording(root, settings);

    // 3. Keep what it found, unless it ran into an error. The cache changes on
    //    every run, and its own `.gitignore` hides what is in it from the walk.
    let cache_dir = adjust_canonicalisation(&cache.dir);
    if let Some(mut inside) = inside
        && let Some(walked) = Walked::new(root, &found)
    {
        inside.contents.retain(|path| !path.starts_with(&cache_dir));
        if let Err(error) =
            snapshot.save(started, &walked, &before, &inside.contents, &inside.kinds)
        {
            let dir = cache.dir.display();
            info!("{}", message!(pipeline_walk_not_saved, dir, error));
        }
    }
    (found, false)
}

/// What a walk found, as kept in the cache: the POSIX paths relative to the
/// root of the source files and the manifests, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Walked {
    /// The source files.
    source_files: Vec<String>,

    /// The manifests.
    manifests: Vec<String>,
}

impl Walked {
    /// What the walk of `root` `found`, or `None` if a path isn't UTF-8.
    fn new(root: &Path, found: &WalkResult) -> Option<Self> {
        let relative = |paths: &[PathBuf]| -> Option<Vec<String>> {
            paths.iter().map(|path| relative_posix(root, path)).collect()
        };
        Some(Self {
            source_files: relative(&found.source_files)?,
            manifests: relative(&found.manifests)?,
        })
    }

    /// What the walk of `root` found.
    fn result(self, root: &Path) -> WalkResult {
        let absolute =
            |paths: Vec<String>| paths.iter().map(|path| absolute_path(root, path)).collect();
        WalkResult {
            source_files: absolute(self.source_files),
            manifests: absolute(self.manifests),
        }
    }
}

impl Cached for Walked {
    const FILE: &'static str = "walk.bin";

    fn encode(&self, encoder: &mut Encoder) {
        for paths in [&self.source_files, &self.manifests] {
            encoder.length(paths.len());
            for path in paths {
                encoder.str(path);
            }
        }
    }

    fn decode(decoder: &mut Decoder<'_>) -> Option<Self> {
        let mut paths = || decoder.list(|decoder| Some(decoder.str()?.to_string()));
        Some(Self { source_files: paths()?, manifests: paths()? })
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
