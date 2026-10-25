//! The session: one project, and everything the stages have found in it.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rustc_hash::FxHashSet;
use thin_vec::ThinVec;
use zenith_cache::CacheSettings;
use zenith_config::{Config, ContractConfig, Reexports};
use zenith_diagnostics::{Report, SourceId, SourceMap, message};
use zenith_graph::{Graph, GraphBuilder};
use zenith_lang::{ParsedFile, Resolved, Suppression, Workspace};
use zenith_utils::{
    fs::{FileSystem, adjust_canonicalisation, disk},
    location::Located,
    path::{absolute_path, relative_posix},
};
use zenith_workspace::{Git, WalkSettings};

use crate::{Backends, FileError, PipelineError, Scope, UnresolvedImport};

/// A project, and what the stages of the pipeline have found in it so far.
///
/// Each field that a stage fills in names it. Until that stage has run, the
/// field is empty.
#[derive(Debug)]
pub struct Session {
    /// The canonical project root.
    pub root: PathBuf,

    /// The file system that the project's files are read from.
    pub file_system: Arc<dyn FileSystem>,

    /// The project's configuration.
    pub config: Config,

    /// The backend of each enabled language, which the stages run.
    pub backends: Backends,

    /// Whether to scan only the files that git tracks, leaving out untracked
    /// ones, as in a pre-commit hook.
    pub tracked_only: bool,

    /// The files that commands report on, every file unless
    /// [`Session::scope_to`] narrows it. Every file is still scanned.
    pub scope: Scope,

    /// Where the [`Parse`](crate::StageKind::Parse) stage keeps its cache, or
    /// `None` to read and parse every file. A new session has none; see
    /// [`Session::cache_settings`]. A session that doesn't read the disk only
    /// reads the cache.
    pub cache: Option<CacheSettings>,

    /// The source files to scan, sorted by path. Found by the
    /// [`Discover`](crate::StageKind::Discover) stage.
    pub files: Vec<SourceFile>,

    /// The workspace packages. Found by the
    /// [`Discover`](crate::StageKind::Discover) stage.
    pub workspace: Workspace,

    /// What the parser found in each file, in the order of `files`. Filled
    /// in by the [`Parse`](crate::StageKind::Parse) stage, and taken by the
    /// [`Graph`](crate::StageKind::Graph) stage.
    pub parsed: Vec<ParsedFile>,

    /// What each import of each file resolved to, in the order of `files` and
    /// of their imports. Filled in by the
    /// [`Resolve`](crate::StageKind::Resolve) stage, and taken by the
    /// [`Graph`](crate::StageKind::Graph) stage.
    pub resolved: Vec<Vec<ResolvedImport>>,

    /// The module tree and import graph. Built by the
    /// [`Graph`](crate::StageKind::Graph) stage; until then it holds only the
    /// root.
    pub graph: Graph,

    /// The import graph through re-exports, for the contracts whose `reexports`
    /// are transparent. Built by the [`Graph`](crate::StageKind::Graph)
    /// stage if a contract is, or if it is
    /// [asked for](Session::build_graph_through_reexports).
    pub graph_through_reexports: Option<Graph>,

    /// Whether the [`Graph`](crate::StageKind::Graph) stage builds the graph
    /// through re-exports: if a contract of the configuration needs it, or if
    /// it was [asked for](Session::build_graph_through_reexports).
    builds_graph_through_reexports: bool,

    /// The source files that could not be read or parsed in full, sorted by
    /// file. Found by the [`Parse`](crate::StageKind::Parse) stage.
    pub file_errors: Vec<FileError>,

    /// The imports that could not be resolved, sorted by file, then in source
    /// order. Imports that match `ignore_unresolved` are left out. Found by
    /// the [`Graph`](crate::StageKind::Graph) stage.
    pub unresolved: Vec<UnresolvedImport>,

    /// The `zenith-ignore` comments of the files that have any, sorted by
    /// file. Kept by the [`Graph`](crate::StageKind::Graph) stage.
    pub suppressions: Vec<FileSuppressions>,

    /// Counts of what the stages found.
    pub stats: Stats,
}

/// The graphs that contracts check against: the import graph, and the graph
/// through re-exports if a contract needs it. They have the same modules, so
/// either resolves a contract's selectors.
#[derive(Debug, Clone, Copy)]
pub struct Graphs<'g> {
    /// The import graph.
    pub graph: &'g Graph,

    /// The import graph through re-exports, if a contract needs it.
    pub through_reexports: Option<&'g Graph>,
}

/// Just the import graph, for contracts that don't see through re-exports.
impl<'g> From<&'g Graph> for Graphs<'g> {
    fn from(graph: &'g Graph) -> Self {
        Self { graph, through_reexports: None }
    }
}

impl<'g> Graphs<'g> {
    /// The graph that the contract `config` checks against.
    ///
    /// Panics if the contract sees through re-exports but the graph through
    /// them wasn't built, which the graph stage does when one does.
    pub fn of(&self, config: &ContractConfig) -> &'g Graph {
        self.seeing(config.reexports)
    }

    /// The graph that a contract that sees re-exports as `reexports` checks
    /// against.
    ///
    /// Panics if it sees through them, but the graph through them wasn't
    /// built.
    pub fn seeing(&self, reexports: Reexports) -> &'g Graph {
        match reexports {
            Reexports::Opaque => self.graph,
            Reexports::Transparent => self
                .through_reexports
                .expect("the graph through re-exports is built for the contracts that need it"),
        }
    }
}

/// What an import resolved to, as the session keeps it until the graph is
/// built.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedImport {
    /// One of the session's source files, by its index in
    /// [`Session::files`]. Most imports resolve to one, which this keeps
    /// without a name of its own.
    File(u32),

    /// Anything else, as the resolver resolved it.
    Other(Resolved),
}

/// A source file to scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Its POSIX path relative to the root, which is its name in the graph.
    pub name: String,

    /// Its language, as the position of its backend in [`Session::backends`].
    pub language: u8,
}

impl SourceFile {
    /// Its absolute path, in the project at `root`. Files keep only their
    /// name, since a project has many, and few stages need their path.
    pub fn path(&self, root: &Path) -> PathBuf {
        absolute_path(root, &self.name)
    }
}

/// The `zenith-ignore` comments of a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSuppressions {
    /// The file, as a POSIX path relative to the root.
    pub file: String,

    /// Its suppressions, in source order. There is at least one.
    pub suppressions: ThinVec<Located<Suppression>>,
}

/// Counts of what the stages found, beyond what the session holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// The dynamic imports and `require` calls whose specifier is not a
    /// literal, which are left out of the graph.
    pub non_literal_imports: usize,

    /// The imports of external packages that aren't installed, so are named
    /// from their specifier alone.
    pub missing_packages: usize,

    /// The source files whose imports came from the parse cache, rather than
    /// the parser.
    pub cached_files: usize,
}

impl Session {
    /// A session for the project on the disk at `root`, which must be
    /// canonical, configured by `config`. Nothing has been found in it yet.
    pub fn new(root: PathBuf, config: Config) -> Self {
        Self::with_file_system(root, disk().clone(), config)
    }

    /// A session for the project at `root` in `file_system`, as
    /// [`Session::new`] makes one for the disk. The root must be canonical
    /// in the file system.
    pub fn with_file_system(
        root: PathBuf,
        file_system: Arc<dyn FileSystem>,
        config: Config,
    ) -> Self {
        let builds_graph_through_reexports = config.needs_graph_through_reexports();
        Self {
            backends: Backends::new(&config, &root, &file_system, builds_graph_through_reexports),
            root,
            file_system,
            config,
            tracked_only: false,
            scope: Scope::default(),
            cache: None,
            files: Vec::new(),
            workspace: Workspace::default(),
            parsed: Vec::new(),
            resolved: Vec::new(),
            graph: GraphBuilder::new().finish(),
            graph_through_reexports: None,
            builds_graph_through_reexports,
            file_errors: Vec::new(),
            unresolved: Vec::new(),
            suppressions: Vec::new(),
            stats: Stats::default(),
        }
    }

    /// Build the graph through re-exports even if no contract of the
    /// configuration needs it, as for contracts that are not in it. Call it
    /// before the [`Parse`](crate::StageKind::Parse) stage runs, which
    /// records what files export for it.
    pub fn build_graph_through_reexports(&mut self) {
        self.builds_graph_through_reexports = true;
        self.backends = Backends::new(&self.config, &self.root, &self.file_system, true);
    }

    /// Whether the [`Graph`](crate::StageKind::Graph) stage builds the graph
    /// through re-exports.
    pub fn needs_graph_through_reexports(&self) -> bool {
        self.builds_graph_through_reexports
    }

    /// A source map that reads the project's files when reports first need
    /// them, with the configuration file in it if it was read from one, and
    /// the id of the configuration file.
    pub fn sources(&self) -> (SourceMap, Option<SourceId>) {
        let mut sources = SourceMap::with_root(&self.root);
        let config = self
            .config
            .file
            .as_ref()
            .map(|file| sources.add_file(&file.path, file.contents.as_str()));
        (sources, config)
    }

    /// The `ignore_unresolved` option of each language whose imports couldn't
    /// all be resolved, as the help about them names it, e.g.
    /// "`[languages.js] ignore_unresolved`".
    fn ignore_unresolved_options(&self) -> String {
        let files: FxHashSet<&str> =
            self.unresolved.iter().map(|import| import.file.as_str()).collect();
        let languages: FxHashSet<usize> = self
            .files
            .iter()
            .filter(|file| files.contains(file.name.as_str()))
            .map(|file| usize::from(file.language))
            .collect();
        (self.config.languages.enabled().enumerate())
            .filter(|(position, _)| languages.contains(position))
            .map(|(_, language)| message!(pipeline_ignore_unresolved, language = language.name))
            .reduce(|first, second| message!(pipeline_either, first, second))
            .unwrap_or_default()
    }

    /// The warning about the imports that couldn't be resolved, listing
    /// where each is if `list` is set, or `None` if there are none.
    pub fn unresolved_report(&self, sources: &mut SourceMap, list: bool) -> Option<Report> {
        if self.unresolved.is_empty() {
            return None;
        }
        let count = self.unresolved.len();
        let mut report = Report::warning(message!(pipeline_unresolved_imports, count));
        if list {
            let lines: Vec<_> = self
                .unresolved
                .iter()
                .map(|import| {
                    let (specifier, reason) = (&import.import.specifier, &import.reason);
                    let location = import.location(sources);
                    format!(
                        "  {}",
                        message!(pipeline_unresolved_entry, location, specifier, reason)
                    )
                })
                .collect();
            report.add_note(lines.join("\n"));
        } else {
            let options = self.ignore_unresolved_options();
            report.add_help(message!(pipeline_unresolved_imports.help, options));
        }
        // A `tsconfig.json` that extends one from a package can't be loaded until
        // the package is installed, and then none of its `paths` resolve.
        let in_tsconfig_errors =
            self.unresolved.iter().filter(|import| import.reason.is_config_error()).count();
        if in_tsconfig_errors > 0 {
            report.add_help(match (in_tsconfig_errors, count) {
                (1, 1) => message!(pipeline_tsconfig_only),
                (in_tsconfig, total) if in_tsconfig == total => message!(pipeline_tsconfig_all),
                (in_tsconfig, _) => message!(pipeline_tsconfig_some, count = in_tsconfig),
            });
        }
        Some(report)
    }

    /// Report only on the files under `paths`, which are relative to the
    /// working directory, and must be inside the root, and on the files that
    /// the branch changed since it left the commit `since`, if it is given:
    /// untracked files too, unless only tracked files are scanned. The
    /// paths need not exist, as a deleted file's importers may still
    /// involve it. No paths and no commit is every file.
    pub fn scope_to(
        &mut self,
        paths: &[PathBuf],
        since: Option<&str>,
    ) -> Result<(), PipelineError> {
        // 1. Name the paths relative to the root.
        let names = paths.iter().map(|path| {
            relative_posix(&self.root, &adjust_canonicalisation(path))
                .ok_or_else(|| PipelineError::OutsideRoot(path.clone()))
        });
        let mut names: Vec<_> = names.collect::<Result<_, _>>()?;

        // 2. Add the files that changed since the commit. If none did, the scope is no
        //    file, rather than every file.
        if let Some(since) = since {
            names.extend(Git::new(&self.root).changed_files(since, !self.tracked_only)?);
        }
        self.scope = match (names.is_empty(), since) {
            (true, None) => Scope::default(),
            _ => Scope::new(names),
        };
        Ok(())
    }

    /// Keep only the problems of the files in the session's scope: the files
    /// that couldn't be read or parsed, and the imports that couldn't be
    /// resolved. The graph keeps every file.
    pub fn narrow(&mut self) {
        let scope = &self.scope;
        self.file_errors.retain(|error| scope.contains(&error.file));
        self.unresolved.retain(|import| scope.contains(&import.file));
    }

    /// The graphs that the contracts check against.
    pub fn graphs(&self) -> Graphs<'_> {
        Graphs { graph: &self.graph, through_reexports: self.graph_through_reexports.as_ref() }
    }

    /// Which files to scan, as the configuration says.
    pub fn walk_settings(&self) -> WalkSettings {
        let files = &self.config.files;
        let scanned = self.config.languages.scanned();
        WalkSettings {
            include: scanned.globs,
            exclude: files.exclude.clone(),
            respect_gitignore: files.respect_gitignore,
            source_extensions: scanned.extensions,
            source_file_names: scanned.file_names,
            manifests: self.backends.manifests(),
        }
    }

    /// Where to keep the parse cache, as the configuration says, for
    /// `version` of zenith, or `None` if the configuration turns it off. The
    /// directory in the configuration is relative to the root.
    pub fn cache_settings(&self, version: &str) -> Option<CacheSettings> {
        let cache = &self.config.cache;
        cache.enabled.then(|| CacheSettings {
            dir: self.root.join(&cache.dir),
            version: version.to_string(),
        })
    }

    /// The path of the baseline file, which the configuration gives relative
    /// to the root.
    pub fn baseline_path(&self) -> PathBuf {
        self.root.join(&self.config.baseline.path)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use zenith_lang_js::JS;

    use super::*;
    use crate::LANGUAGES;

    /// A session for a project at `/repo` with the configuration `toml`.
    fn session(toml: &str) -> Session {
        Session::new(PathBuf::from("/repo"), Config::parse(toml, LANGUAGES).unwrap())
    }

    #[test]
    fn a_new_session_has_found_nothing() {
        let session = session("");
        assert!(session.files.is_empty() && session.parsed.is_empty());
        assert!(session.resolved.is_empty() && session.workspace.packages.is_empty());
        assert_eq!(session.graph.node_count(), 1);
        assert!(session.file_errors.is_empty() && session.unresolved.is_empty());
        assert!(session.suppressions.is_empty());
    }

    #[test]
    fn defaults_scan_every_source_file() {
        let session = session("");
        let walk = session.walk_settings();
        assert!(walk.include.is_empty() && walk.exclude.is_empty());
        assert!(walk.respect_gitignore);
        assert_eq!(walk.source_extensions, JS.source_extensions);
        assert_eq!(walk.manifests, ["package.json"]);
        assert_eq!(session.baseline_path(), Path::new("/repo/zenith.baseline.toml"));
        assert_eq!(
            session.cache_settings("1.0.0"),
            Some(CacheSettings {
                dir: PathBuf::from("/repo/.zenith_cache"),
                version: "1.0.0".to_string()
            })
        );
        assert!(session.cache.is_none());
    }

    #[test]
    fn the_configuration_replaces_the_defaults() {
        let session = session(include_str!("fixtures/replaced-defaults.toml"));
        assert_eq!(session.baseline_path(), Path::new("/repo/lint/baseline.toml"));
        let cache = session.cache_settings("1.0.0").unwrap();
        assert_eq!(cache.dir, Path::new("/repo/.zenith"));
        let globs = |globs: &[zenith_utils::glob::Glob]| -> Vec<String> {
            globs.iter().map(|glob| glob.as_str().to_string()).collect()
        };
        let walk = session.walk_settings();
        assert_eq!(globs(&walk.include), ["web/**"]);
        // The language scans the files that `include` says, not by extension.
        assert!(walk.source_extensions.is_empty());
        assert_eq!(globs(&walk.exclude), ["**/*.test.ts"]);
        assert!(!walk.respect_gitignore);
    }

    #[test]
    fn the_cache_can_be_turned_off() {
        assert_eq!(session("[cache]\nenabled = false\n").cache_settings("1.0.0"), None);
    }
}
