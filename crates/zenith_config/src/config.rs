//! The validated configuration of a zenith project, and finding and loading
//! its files.
//!
//! These types are what [`Config::parse`] produces: defaults are filled in,
//! and selectors, globs, layers and ignored imports are checked and compiled.
//! Resolving selectors against the project is left to the contracts.

use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zenith_diagnostics::{ReportKind, SourceId, SourceMap};
use zenith_graph::KindSet;
use zenith_utils::{fs::FileSystem, glob::Glob, path::relative_posix};

use crate::{
    contracts::{ContractConfig, Reexports, Severity},
    error::{ConfigError, ConfigErrorKind, InvalidFile, LoadError},
    language::{Language, LanguageOptions, Languages},
    parse::{self, ParsedRoot},
    parts::{CONFIG_FILE_NAME, PartSearch},
};

/// The configuration of a zenith project.
#[derive(Debug, Clone)]
pub struct Config {
    /// Which files to scan for imports.
    pub files: FilesConfig,

    /// The languages to scan, each with its own options.
    pub languages: LanguagesConfig,

    /// Defaults that apply to every contract.
    pub analysis: AnalysisConfig,

    /// The baseline of known violations.
    pub baseline: BaselineConfig,

    /// The cache of what zenith found in each file on earlier runs.
    pub cache: CacheConfig,

    /// The contracts, in the order of the file.
    pub contracts: Vec<ContractConfig>,

    /// The files the configuration was loaded from, so that errors found once
    /// it is used, such as a selector that matches nothing, can be shown in
    /// context: the root's first, then the files it includes and its
    /// members' files, then the files that those include. Empty for the
    /// defaults, or a configuration parsed from text.
    pub config_files: Vec<ConfigFile>,
}

/// A configuration file, as it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    /// The path it was read from.
    pub path: PathBuf,

    /// Its contents.
    pub contents: String,
}

impl ConfigFile {
    /// Read the configuration file at `path` from `file_system`.
    fn read(file_system: &dyn FileSystem, path: &Path) -> Result<Self, LoadError> {
        match file_system.read_text(path) {
            Ok(contents) => Ok(Self { path: path.to_path_buf(), contents }),
            Err(error) => Err(LoadError::Read { path: path.to_path_buf(), error }),
        }
    }
}

/// The error of a configuration whose files, `read`, each with the errors in
/// it, are invalid.
fn invalid(read: Vec<(ConfigFile, Vec<ConfigError>)>) -> LoadError {
    LoadError::invalid(
        read.into_iter().map(|(ConfigFile { path, contents }, errors)| InvalidFile {
            path,
            contents,
            errors,
        }),
    )
}

/// The configuration files in a source map, as [`Config::add_sources`] adds
/// them.
#[derive(Debug, Clone, Default)]
pub struct ConfigSources(Vec<SourceId>);

impl ConfigSources {
    /// The source of the configuration file that `contract` is written in,
    /// if it was read from one.
    pub fn of(&self, contract: &ContractConfig) -> Option<SourceId> {
        self.0.get(contract.file).copied()
    }
}

impl Config {
    /// The configuration of a project without a configuration file, which
    /// enables the `languages` that are enabled by default.
    pub fn defaults(languages: Languages) -> Self {
        Self {
            files: FilesConfig::default(),
            languages: LanguagesConfig::defaults(languages),
            analysis: AnalysisConfig::default(),
            baseline: BaselineConfig::default(),
            cache: CacheConfig::default(),
            contracts: Vec::new(),
            config_files: Vec::new(),
        }
    }

    /// Find the configuration file of the project that `start` is in, in
    /// `file_system`: the nearest in `start` or one of its ancestors, or, if
    /// that is a member's, the root's that lists it in `[workspace] members`.
    /// A relative `start` is taken from the current directory.
    pub fn discover(file_system: &dyn FileSystem, start: &Path) -> Option<PathBuf> {
        let nearest = file_system.find_in_ancestors(start, CONFIG_FILE_NAME)?;
        let dir = nearest.parent()?;
        let root = dir.ancestors().skip(1).map(|ancestor| ancestor.join(CONFIG_FILE_NAME)).find(
            |config| {
                let member = config.parent().and_then(|root| relative_posix(root, dir));
                let contents = file_system.read_text(config).ok();
                member
                    .zip(contents)
                    .is_some_and(|(member, contents)| parse::lists_member(&contents, &member))
            },
        );
        Some(root.unwrap_or(nearest))
    }

    /// Read the root's configuration file, at `path`, from `file_system`,
    /// and the files it is split over, under the project `root`: the files
    /// it includes, its members' files, and the files that those include.
    /// Their globs are matched against the files that `walk` finds. The root's
    /// is parsed as [`Config::parse`] parses it, and each of the others
    /// with the options of the file that includes it, or the root's, as
    /// their defaults, adding its contracts. The configuration keeps the
    /// files, in [`Config::config_files`].
    pub fn load(
        file_system: &dyn FileSystem,
        path: &Path,
        root: &Path,
        languages: Languages,
        walk: &dyn ProjectWalk,
    ) -> Result<Self, LoadError> {
        // 1. Read the root's file, and parse it.
        let file = ConfigFile::read(file_system, path)?;
        let (parsed, errors) = parse::parse_root(&file.contents, languages);
        let mut read = vec![(file, errors)];
        let Some(ParsedRoot { mut config, globs, languages: root_languages }) = parsed else {
            return Err(invalid(read));
        };

        // 2. Read the files it includes, and its members' files, then those that they
        //    include, and parse each with the options of the file that includes it, or
        //    the root's, as their defaults. A file's own `[files] exclude` applies to
        //    those it includes too.
        let files = config.files.clone();
        let walk = move || walk.files(file_system, root, &files);
        let mut search = PartSearch::new(file_system, root, path, walk);
        let found = search.find(&globs, None, &[], &mut read[0].1);
        let mut pending: VecDeque<_> =
            found.into_iter().map(|part| (part, config.analysis.clone(), Vec::new())).collect();
        while let Some((part, analysis, mut exclude)) = pending.pop_front() {
            let file = ConfigFile::read(file_system, &part.path)?;
            let (member, file_index) = (part.member.as_deref(), read.len());
            let (parsed, mut errors) = parse::parse_part(
                &file.contents,
                &root_languages,
                &analysis,
                file_index,
                member,
                languages,
            );
            if let Some((parsed, globs)) = parsed {
                config.files.exclude.extend(parsed.exclude.iter().cloned());
                config.contracts.extend(parsed.contracts);
                exclude.extend(parsed.exclude);
                let found = search.find(&globs, member, &exclude, &mut errors);
                let analysis = &parsed.analysis;
                pending.extend(
                    found.into_iter().map(|part| (part, analysis.clone(), exclude.clone())),
                );
            }
            read.push((file, errors));
        }

        // 3. Every contract must have an id of its own, whichever file it is in.
        let mut first: HashMap<&str, usize> = HashMap::new();
        let mut duplicates = Vec::new();
        for contract in &config.contracts {
            let Some(&file) = first.get(contract.id.as_str()) else {
                first.insert(&contract.id, contract.file);
                continue;
            };
            let path = &read[file].0.path;
            let first = path.strip_prefix(root).unwrap_or(path).display().to_string();
            let kind = ConfigErrorKind::DuplicateContract { id: contract.id.clone(), first };
            duplicates.push((
                contract.file,
                ConfigError { kind, contract: None, range: Some(contract.range) },
            ));
        }
        for (file, error) in duplicates {
            read[file].1.push(error);
        }

        // 4. Keep the files, or else fail with the errors in them.
        if read.iter().any(|(_, errors)| !errors.is_empty()) {
            return Err(invalid(read));
        }
        config.config_files = read.into_iter().map(|(file, _)| file).collect();
        Ok(config)
    }

    /// Parse and validate the contents of a configuration file. On failure,
    /// returns the errors, in the order of the file: the first error in the
    /// structure of each table and contract, such as an unknown key, and
    /// every invalid value, such as a selector. The file may enable any of
    /// the `languages`. The files that it includes, and its members' files,
    /// are not read: [`Config::load`] reads them.
    pub fn parse(contents: &str, languages: Languages) -> Result<Self, Vec<ConfigError>> {
        parse::parse(contents, languages)
    }

    /// Add the configuration files to `sources`, for reports to show what is
    /// wrong in them in context.
    pub fn add_sources(&self, sources: &mut SourceMap) -> ConfigSources {
        let files = self.config_files.iter();
        ConfigSources(
            files.map(|file| sources.add_file(&file.path, file.contents.as_str())).collect(),
        )
    }

    /// Parse and validate contracts written as in a configuration file, in
    /// `[contracts]` tables, as contracts of this configuration: they default
    /// to the options of its `[analysis]`, and may use the selector schemes of
    /// the languages it enables, of the `languages` it may enable. On failure,
    /// returns the errors, as [`Config::parse`] does.
    pub fn parse_contracts(
        &self,
        contents: &str,
        languages: Languages,
    ) -> Result<Vec<ContractConfig>, Vec<ConfigError>> {
        parse::parse_contracts(contents, self, languages)
    }

    /// The contract with the given id.
    pub fn contract(&self, id: &str) -> Option<&ContractConfig> {
        self.contracts.iter().find(|contract| contract.id == id)
    }

    /// The contract that `name`, in a `zenith-ignore` comment of `file`, a
    /// POSIX path relative to the root, names: in the files of a member, the
    /// member's contract that is written with that id, of the innermost
    /// member that has one, or else the contract with that id.
    pub fn contract_named(&self, name: &str, file: &str) -> Option<&ContractConfig> {
        let in_member = |contract: &&ContractConfig| {
            contract.member.as_deref().is_some_and(|member| {
                let rest = file.strip_prefix(member);
                rest.is_some_and(|rest| rest.starts_with('/'))
                    && contract.id.strip_prefix(&ContractConfig::member_prefix(member))
                        == Some(name)
            })
        };
        let innermost = |contract: &&ContractConfig| contract.member.as_ref().map(String::len);
        let member = self.contracts.iter().filter(in_member).max_by_key(innermost);
        member.or_else(|| self.contract(name))
    }

    /// The root's configuration file, if the configuration was read from one.
    pub fn root_file(&self) -> Option<&ConfigFile> {
        self.config_files.first()
    }

    /// Whether any contract sees through re-exports, so needs the graph
    /// through them.
    pub fn needs_graph_through_reexports(&self) -> bool {
        self.contracts.iter().any(|contract| contract.reexports == Reexports::Transparent)
    }
}

/// The walk of a project that the globs of its configuration files are
/// matched against: the caller's, as walking the project is the pipeline's.
pub trait ProjectWalk {
    /// The files of the project at `root`, in `file_system`, that the walk
    /// finds, which skips what `files` skips.
    fn files(&self, file_system: &dyn FileSystem, root: &Path, files: &FilesConfig)
    -> Vec<PathBuf>;
}

/// Which files to scan for imports, whatever their language.
#[derive(Debug, Clone)]
pub struct FilesConfig {
    /// Globs of files and directories to skip, on top of the default
    /// excludes.
    pub exclude: Vec<Glob>,

    /// Whether to skip the files that `.gitignore`, `.ignore` and the global
    /// git excludes ignore.
    pub respect_gitignore: bool,
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self { exclude: Vec::new(), respect_gitignore: true }
    }
}

/// The languages to scan, each with its options. A language is enabled when
/// its table is written; without a `[languages]` table, those that are
/// enabled by default are.
#[derive(Debug, Clone)]
pub struct LanguagesConfig {
    /// The enabled languages, in the order of the registry, with their
    /// options.
    enabled: Vec<(&'static Language, Arc<dyn LanguageOptions>)>,
}

impl LanguagesConfig {
    /// The `languages` that are enabled by default, with their defaults.
    pub fn defaults(languages: Languages) -> Self {
        let enabled = languages.iter().filter(|language| language.enabled_by_default);
        Self { enabled: enabled.map(|language| (language, (language.defaults)())).collect() }
    }

    /// The `enabled` languages, with their options.
    pub(crate) fn new(enabled: Vec<(&'static Language, Arc<dyn LanguageOptions>)>) -> Self {
        Self { enabled }
    }

    /// Whether `language` is enabled.
    pub fn is_enabled(&self, language: &Language) -> bool {
        self.enabled().any(|enabled| enabled == language)
    }

    /// The enabled languages, in the order of the registry.
    pub fn enabled(&self) -> impl Iterator<Item = &'static Language> {
        self.enabled.iter().map(|(language, _)| *language)
    }

    /// The enabled languages with their options, in the order of the
    /// registry.
    pub fn iter(&self) -> impl Iterator<Item = (&'static Language, &dyn LanguageOptions)> {
        self.enabled.iter().map(|(language, options)| (*language, options.as_ref()))
    }

    /// The position, among the [enabled](Self::enabled) languages, of the
    /// language whose source file `name` is: the first whose `include`
    /// matches it, or that scans by extension and file name and has its
    /// extension or its file name. `None` for a file of no enabled language.
    pub fn language_of(&self, name: &str) -> Option<usize> {
        let file_name = name.rsplit('/').next().unwrap_or(name);
        let extension = file_name.rsplit_once('.').map(|(_, extension)| extension);
        self.iter().position(|(language, options)| match options.include() {
            Some(include) => include.iter().any(|glob| glob.is_match(name)),
            None => {
                extension.is_some_and(|extension| language.source_extensions.contains(&extension))
                    || language.source_file_names.contains(&file_name)
            }
        })
    }

    /// The options of the enabled language whose options are a `T`, or
    /// `None` if it isn't enabled.
    pub fn options<T: LanguageOptions>(&self) -> Option<&T> {
        self.enabled.iter().find_map(|(_, options)| (options.as_ref() as &dyn Any).downcast_ref())
    }

    /// The extensions, without the `.`, that a path selector may leave off a
    /// file's path: those of every enabled language.
    pub fn optional_extensions(&self) -> Vec<&'static str> {
        union(self.enabled().map(|language| language.optional_extensions))
    }

    /// The extensions, without the `.`, of the files that are one module with
    /// the directory beside them of the same name: those of every enabled
    /// language.
    pub fn directory_modules(&self) -> Vec<&'static str> {
        union(self.enabled().map(|language| language.directory_modules))
    }

    /// Which files to scan: the globs that the enabled languages set as their
    /// `include`, and the source extensions and file names of those that
    /// leave it to the default.
    pub fn scanned(&self) -> Scanned {
        let mut globs = Vec::new();
        let mut extensions = Vec::new();
        let mut file_names = Vec::new();
        for (language, options) in &self.enabled {
            match options.include() {
                Some(include) => globs.extend(include.iter().cloned()),
                None => {
                    extensions.push(language.source_extensions);
                    file_names.push(language.source_file_names);
                }
            }
        }
        Scanned { globs, extensions: union(extensions), file_names: union(file_names) }
    }
}

/// Which files the enabled languages scan: those that match one of the
/// globs, and those with one of the extensions or file names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scanned {
    /// The `include` globs of the languages that set them.
    pub globs: Vec<Glob>,

    /// The source extensions, without the `.`, of the languages that don't.
    pub extensions: Vec<&'static str>,

    /// The source file names of the languages that don't, such as
    /// `terragrunt.hcl`.
    pub file_names: Vec<&'static str>,
}

/// The values of `lists`, in order, without repeats.
fn union<'a>(lists: impl IntoIterator<Item = &'a [&'static str]>) -> Vec<&'static str> {
    let mut union: Vec<&'static str> = Vec::new();
    for value in lists.into_iter().flatten() {
        if !union.contains(value) {
            union.push(value);
        }
    }
    union
}

/// Defaults that apply to every contract.
#[derive(Debug, Clone, Default)]
pub struct AnalysisConfig {
    /// The import kinds that contracts ignore, unless they set their own.
    pub exclude_kinds: KindSet,

    /// How contracts see imports through re-exports, unless they say.
    pub reexports: Reexports,
}

/// The baseline of known violations.
#[derive(Debug, Clone)]
pub struct BaselineConfig {
    /// The path of the baseline file, relative to the project root.
    pub path: String,

    /// How to report baseline entries whose import no longer exists.
    pub stale: StaleLevel,
}

/// The default path of the baseline file.
pub(crate) const DEFAULT_BASELINE_PATH: &str = "zenith.baseline.toml";

impl BaselineConfig {
    /// How to report the stale entries of a contract of `severity`: as errors
    /// if it is of `error` severity and stale entries are errors, and as
    /// warnings otherwise.
    pub fn stale_kind(&self, severity: Severity) -> ReportKind {
        match (severity, self.stale) {
            (Severity::Error, StaleLevel::Error) => ReportKind::Error,
            _ => ReportKind::Warning,
        }
    }
}

impl Default for BaselineConfig {
    fn default() -> Self {
        Self { path: DEFAULT_BASELINE_PATH.to_string(), stale: StaleLevel::default() }
    }
}

/// The cache of what zenith found in each file on earlier runs, which lets
/// it skip reading and parsing the files that haven't changed since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    /// Whether to use the cache.
    pub enabled: bool,

    /// The directory to keep the cache in, relative to the project root.
    pub dir: String,
}

/// The default directory of the cache.
pub(crate) const DEFAULT_CACHE_DIR: &str = ".zenith_cache";

impl Default for CacheConfig {
    fn default() -> Self {
        Self { enabled: true, dir: DEFAULT_CACHE_DIR.to_string() }
    }
}

/// How to report baseline entries whose import no longer exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StaleLevel {
    /// Stale entries fail the check, so the baseline can only shrink.
    #[default]
    Error,

    /// Stale entries are reported as warnings.
    Warn,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::fs::OsFileSystem;

    use super::*;
    use crate::testing::{LANGUAGES, TestWalk};

    #[test]
    fn discovers_the_nearest_config_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("web/src")).unwrap();
        fs::write(root.join(CONFIG_FILE_NAME), "").unwrap();
        assert_eq!(
            Config::discover(&OsFileSystem, &root.join("web/src")),
            Some(root.join(CONFIG_FILE_NAME))
        );
    }

    /// A project with the files at `paths`, each with its contents.
    fn project(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (path, contents) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        dir
    }

    /// A project whose configuration is split over a file that the root
    /// includes, a member's file and one that the member includes, with an
    /// unlisted project inside it.
    fn split_project() -> TempDir {
        project(&[
            (CONFIG_FILE_NAME, include_str!("parse/fixtures/split-root.toml")),
            ("zenith/rules.toml", include_str!("parse/fixtures/split-included.toml")),
            ("apps/web/zenith.toml", include_str!("parse/fixtures/member.toml")),
            (
                "apps/web/zenith/extra.toml",
                include_str!("parse/fixtures/split-member-included.toml"),
            ),
            ("apps/web/src/a.tst", ""),
            ("apps/admin/src/a.tst", ""),
            ("tools/zenith.toml", ""),
        ])
    }

    #[test]
    fn loads_the_files_that_the_root_s_is_split_over() {
        let dir = split_project();
        let root = dir.path();
        let config =
            Config::load(&OsFileSystem, &root.join(CONFIG_FILE_NAME), root, LANGUAGES, &TestWalk);
        let config = config.unwrap();
        let ids: Vec<_> = config.contracts.iter().map(|contract| contract.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "root",
                "features",
                "included",
                "apps.web.layers",
                "apps.web.features",
                "apps.web.no-ui",
                "apps.web.extra"
            ]
        );
        let files: Vec<_> = config.config_files.iter().map(|file| file.path.clone()).collect();
        assert_eq!(
            files,
            [
                root.join(CONFIG_FILE_NAME),
                root.join("zenith/rules.toml"),
                root.join("apps/web/zenith.toml"),
                root.join("apps/web/zenith/extra.toml")
            ]
        );
        let file_of = |id| config.contract(id).unwrap().file;
        assert_eq!((file_of("root"), file_of("included"), file_of("apps.web.layers")), (0, 1, 2));
        // A file that a member includes is part of the member.
        let extra = config.contract("apps.web.extra").unwrap();
        assert_eq!((extra.file, extra.member.as_deref()), (3, Some("apps/web")));
        // Each file's options default to those of the file that includes it.
        let reexports = |id| config.contract(id).unwrap().reexports;
        assert_eq!(reexports("included"), Reexports::Transparent);
        assert_eq!(reexports("apps.web.extra"), Reexports::Opaque);
        assert!(config.files.exclude.iter().any(|glob| glob.is_match("apps/web/generated/a.tst")));
    }

    #[test]
    fn a_member_s_files_name_its_contracts_by_their_own_ids() {
        let dir = split_project();
        let root = dir.path();
        let config =
            Config::load(&OsFileSystem, &root.join(CONFIG_FILE_NAME), root, LANGUAGES, &TestWalk);
        let config = config.unwrap();
        let named =
            |name, file| config.contract_named(name, file).map(|contract| contract.id.as_str());
        assert_eq!(named("layers", "apps/web/src/a.tst"), Some("apps.web.layers"));
        assert_eq!(named("apps.web.layers", "apps/admin/src/a.tst"), Some("apps.web.layers"));
        assert_eq!(named("layers", "apps/admin/src/a.tst"), None);
        assert_eq!(named("layers", "apps/webapp/src/a.tst"), None);
        // The member's contract, rather than the root's of the same id.
        assert_eq!(named("features", "apps/web/src/a.tst"), Some("apps.web.features"));
        assert_eq!(named("features", "apps/admin/src/a.tst"), Some("features"));
        assert_eq!(named("root", "apps/web/src/a.tst"), Some("root"));
    }

    #[test]
    fn discovers_the_root_s_config_file_from_a_member() {
        let dir = split_project();
        let root = dir.path();
        let discover = |dir: &str| Config::discover(&OsFileSystem, &root.join(dir)).unwrap();
        assert_eq!(discover("apps/web/src"), root.join(CONFIG_FILE_NAME));
        assert_eq!(discover("apps/admin/src"), root.join(CONFIG_FILE_NAME));
        // A project that the root doesn't list is a project of its own.
        assert_eq!(discover("tools"), root.join("tools").join(CONFIG_FILE_NAME));
    }

    #[test]
    fn a_loaded_config_keeps_its_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let contents = "[baseline]\nstale = \"warn\"\n";
        fs::write(&path, contents).unwrap();
        let files = Config::load(&OsFileSystem, &path, dir.path(), LANGUAGES, &TestWalk)
            .unwrap()
            .config_files;
        assert_eq!(files, [ConfigFile { path, contents: contents.to_string() }]);
        assert!(Config::parse(contents, LANGUAGES).unwrap().config_files.is_empty());
    }

    #[test]
    fn loads_a_config_with_a_byte_order_mark() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(&path, "\u{feff}[baseline]\nstale = \"warn\"\n").unwrap();
        assert_eq!(
            Config::load(&OsFileSystem, &path, dir.path(), LANGUAGES, &TestWalk)
                .unwrap()
                .baseline
                .stale,
            StaleLevel::Warn
        );
    }

    #[test]
    fn tells_unreadable_from_invalid_configs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let error =
            Config::load(&OsFileSystem, &path, dir.path(), LANGUAGES, &TestWalk).unwrap_err();
        assert!(matches!(&error, LoadError::Read { path: p, .. } if *p == path), "{error:?}");
        assert!(error.to_string().starts_with("could not read `"), "{error}");

        let contents = "[files]\nexclud = []\n\n[contracts.a]\ntype = \"layer\"\n";
        fs::write(&path, contents).unwrap();
        let error =
            Config::load(&OsFileSystem, &path, dir.path(), LANGUAGES, &TestWalk).unwrap_err();
        let LoadError::Invalid { files } = &error else {
            panic!("expected an invalid config, got {error:?}");
        };
        let [InvalidFile { contents: read, errors, .. }] = files.as_slice() else {
            panic!("expected one invalid file, got {files:?}");
        };
        assert_eq!(read, contents);
        assert_eq!(errors.len(), 2);

        // The errors are shown in context.
        let path = path.display();
        let expected = [
            "error: unknown key `exclud`".to_string(),
            format!(" --> {path}:2:1"),
            "  |".to_string(),
            "2 | exclud = []".to_string(),
            "  | ^^^^^^".to_string(),
            "  |".to_string(),
            "  = help: did you mean `exclude`?".to_string(),
            String::new(),
            "error: unknown contract type `layer`".to_string(),
            format!(" --> {path}:5:8"),
            "  |".to_string(),
            "5 | type = \"layer\"".to_string(),
            "  |        ^^^^^^^".to_string(),
            "  |".to_string(),
            "  = note: in contract `a`".to_string(),
            "  = help: did you mean `layers`?".to_string(),
        ];
        assert_eq!(error.to_string(), expected.join("\n"));
    }
}
