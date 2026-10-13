//! The session: one project, and everything the stages have found in it.

use std::path::{Path, PathBuf};

use thin_vec::ThinVec;
use zenith_config::{Config, SOURCE_EXTENSIONS, Tsconfig};
use zenith_diagnostics::{Report, SourceId, SourceMap};
use zenith_graph::{Graph, GraphBuilder};
use zenith_parse::{ParsedFile, Suppression};
use zenith_resolve::{Resolved, Resolver, ResolverError, ResolverSettings, TsconfigMode};
use zenith_utils::{location::Located, path::absolute_path, printing::plural};
use zenith_workspace::{WalkSettings, Workspace};

use crate::{FileError, UnresolvedImport};

/// A project, and what the stages of the pipeline have found in it so far.
///
/// Each field that a stage fills in names it. Until that stage has run, the
/// field is empty.
#[derive(Debug)]
pub struct Session {
    /// The canonical project root.
    pub root: PathBuf,

    /// The project's configuration.
    pub config: Config,

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
}

impl Session {
    /// A session for the project at `root`, which must be canonical,
    /// configured by `config`. Nothing has been found in it yet.
    pub fn new(root: PathBuf, config: Config) -> Self {
        Self {
            root,
            config,
            files: Vec::new(),
            workspace: Workspace::default(),
            parsed: Vec::new(),
            resolved: Vec::new(),
            graph: GraphBuilder::new().finish(),
            file_errors: Vec::new(),
            unresolved: Vec::new(),
            suppressions: Vec::new(),
            stats: Stats::default(),
        }
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

    /// The warning about the imports that couldn't be resolved, listing
    /// where each is if `list` is set, or `None` if there are none.
    pub fn unresolved_report(&self, sources: &mut SourceMap, list: bool) -> Option<Report> {
        if self.unresolved.is_empty() {
            return None;
        }
        let title = format!("{} could not be resolved", plural(self.unresolved.len(), "import"));
        let mut report = Report::warning(title);
        if list {
            let lines: Vec<_> = self
                .unresolved
                .iter()
                .map(|import| {
                    let (specifier, reason) = (&import.import.specifier, &import.reason);
                    format!("  {}  `{specifier}`  ({reason})", import.location(sources))
                })
                .collect();
            report.add_note(lines.join("\n"));
        } else {
            report.add_help(
                "use `--show-unresolved` to list them, or `[resolve] ignore_unresolved` to silence expected ones",
            );
        }
        // A `tsconfig.json` that extends one from a package can't be loaded until
        // the package is installed, and then none of its `paths` resolve.
        let in_tsconfig_errors =
            self.unresolved.iter().filter(|import| import.reason.is_tsconfig_error()).count();
        if in_tsconfig_errors > 0 {
            let which = match (in_tsconfig_errors, self.unresolved.len()) {
                (1, 1) => "it is in a file".to_string(),
                (count, total) if count == total => "they are all in files".to_string(),
                (1, _) => "1 of them is in a file".to_string(),
                (count, _) => format!("{count} of them are in files"),
            };
            report.add_help(format!(
                "{which} whose `tsconfig.json` couldn't be loaded; if it extends one from a \
                 package, install the dependencies first"
            ));
        }
        Some(report)
    }

    /// Which files to scan, as the configuration says.
    pub fn walk_settings(&self) -> WalkSettings {
        let files = &self.config.files;
        WalkSettings {
            include: files.include.clone(),
            exclude: files.exclude.clone(),
            respect_gitignore: files.respect_gitignore,
            source_extensions: SOURCE_EXTENSIONS,
        }
    }

    /// How to resolve specifiers, as the configuration says, with the
    /// workspace packages found so far. Paths in the configuration are
    /// relative to the root.
    pub fn resolver_settings(&self) -> ResolverSettings {
        let resolve = &self.config.resolve;
        let mut settings = ResolverSettings::new(self.root.clone());
        settings.tsconfig = match &resolve.tsconfig {
            Tsconfig::Auto => TsconfigMode::Auto,
            Tsconfig::Off => TsconfigMode::Off,
            Tsconfig::File(path) => TsconfigMode::File(self.root.join(path)),
        };
        if let Some(conditions) = &resolve.conditions {
            settings.conditions = conditions.clone();
        }
        if let Some(main_fields) = &resolve.main_fields {
            settings.main_fields = main_fields.clone();
        }
        if let Some(extensions) = &resolve.extensions {
            settings.extensions = extensions.clone();
        }
        settings.aliases = resolve
            .alias
            .iter()
            .map(|(alias, path)| (alias.clone(), self.root.join(path)))
            .collect();
        settings.workspace_packages = self
            .workspace
            .packages
            .iter()
            .map(|package| (package.name.clone(), package.path.clone()))
            .collect();
        settings.ignore_unresolved = resolve.ignore_unresolved.clone();
        settings
    }

    /// The path of the baseline file, which the configuration gives relative
    /// to the root.
    pub fn baseline_path(&self) -> PathBuf {
        self.root.join(&self.config.baseline.path)
    }

    /// A resolver configured by [`Session::resolver_settings`]. Run the
    /// [`Discover`](crate::StageKind::Discover) stage first, so that workspace
    /// packages resolve without `node_modules`.
    pub fn resolver(&self) -> Result<Resolver, ResolverError> {
        Resolver::new(self.resolver_settings())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use zenith_workspace::WorkspacePackage;

    use super::*;

    /// A session for a project at `/repo` with the configuration `toml`.
    fn session(toml: &str) -> Session {
        Session::new(PathBuf::from("/repo"), Config::parse(toml).unwrap())
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
    fn defaults_scan_every_source_file_and_resolve_like_typescript() {
        let session = session("");
        let walk = session.walk_settings();
        assert!(walk.include.is_none() && walk.exclude.is_empty());
        assert!(walk.respect_gitignore);
        assert_eq!(walk.source_extensions, SOURCE_EXTENSIONS);

        let defaults = ResolverSettings::new(PathBuf::from("/repo"));
        let settings = session.resolver_settings();
        assert_eq!(settings.root, Path::new("/repo"));
        assert_eq!(settings.tsconfig, TsconfigMode::Auto);
        assert_eq!(settings.conditions, defaults.conditions);
        assert_eq!(settings.main_fields, defaults.main_fields);
        assert_eq!(settings.extensions, defaults.extensions);
        assert!(settings.aliases.is_empty() && settings.ignore_unresolved.is_empty());
        assert!(settings.workspace_packages.is_empty());
        assert_eq!(session.baseline_path(), Path::new("/repo/zenith.baseline.toml"));
    }

    #[test]
    fn the_configuration_replaces_the_defaults() {
        let session = session(
            r#"
            [files]
            include = ["web/**"]
            exclude = ["**/*.test.ts"]
            respect_gitignore = false

            [resolve]
            tsconfig = "web/tsconfig.json"
            conditions = ["browser", "import"]
            main_fields = ["main"]
            extensions = [".ts", ".js"]
            alias = { "types" = "web/src/types", "~" = "web" }
            ignore_unresolved = ["virtual:*"]

            [baseline]
            path = "lint/baseline.toml"
            "#,
        );
        assert_eq!(session.baseline_path(), Path::new("/repo/lint/baseline.toml"));
        let globs = |globs: &[zenith_utils::glob::Glob]| -> Vec<String> {
            globs.iter().map(|glob| glob.as_str().to_string()).collect()
        };
        let walk = session.walk_settings();
        assert_eq!(globs(walk.include.as_deref().unwrap()), ["web/**"]);
        assert_eq!(globs(&walk.exclude), ["**/*.test.ts"]);
        assert!(!walk.respect_gitignore);

        let settings = session.resolver_settings();
        assert_eq!(settings.tsconfig, TsconfigMode::File(PathBuf::from("/repo/web/tsconfig.json")));
        assert_eq!(settings.conditions, ["browser", "import"]);
        assert_eq!(settings.main_fields, ["main"]);
        assert_eq!(settings.extensions, [".ts", ".js"]);
        // Aliases keep the order of the file, and point into the root.
        assert_eq!(
            settings.aliases,
            [
                ("types".to_string(), PathBuf::from("/repo/web/src/types")),
                ("~".to_string(), PathBuf::from("/repo/web")),
            ]
        );
        let patterns: Vec<_> =
            settings.ignore_unresolved.iter().map(|pattern| pattern.as_str()).collect();
        assert_eq!(patterns, ["virtual:*"]);
    }

    #[test]
    fn tsconfig_files_can_be_turned_off() {
        let settings = session("[resolve]\ntsconfig = \"off\"\n").resolver_settings();
        assert_eq!(settings.tsconfig, TsconfigMode::Off);
    }

    #[test]
    fn the_resolver_knows_the_workspace_packages() {
        let mut session = session("");
        session.workspace.packages.push(WorkspacePackage {
            name: "@acme/ui".to_string(),
            dir: "packages/ui".to_string(),
            path: PathBuf::from("/repo/packages/ui"),
        });
        assert_eq!(
            session.resolver_settings().workspace_packages,
            [("@acme/ui".to_string(), PathBuf::from("/repo/packages/ui"))]
        );
    }
}
