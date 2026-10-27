//! Parsing `zenith.toml` and validating it into a [`Config`].
//!
//! The file is parsed into a TOML document that keeps the location of every
//! key and value. Each table is deserialised from it on its own, and so is
//! each contract: its `type` picks the table to deserialise the rest of it
//! into, so that its errors point at the offending key. Deserialising a table
//! stops at its first error, but the other tables still report theirs. Every
//! value is then validated, and the errors are collected in the order of the
//! file.
//!
//! A file that is part of the root's configuration, one that it includes or
//! a member's, is parsed on its own too, with the root's options as its
//! defaults. A member's paths, and those of the files it includes, are read
//! under its directory, and its contracts' ids start with it.
//!
//! The parser is split by what it parses: `tables` parses the global tables,
//! `languages` the languages, `contracts` the contracts, and `selector` and
//! `layer` the strings that selectors, ignored imports and layers are written
//! as.

mod contracts;
mod languages;
mod layer;
mod selector;
mod tables;
#[cfg(test)]
mod tests;

pub use languages::LanguageTable;
use serde::Deserialize;
use toml::{
    Spanned,
    de::{DeTable, DeValue, Deserializer},
};
use zenith_diagnostics::DiagnosticStore;
use zenith_utils::{
    glob::{Glob, GlobError},
    location::ByteRange,
};

use crate::{
    config::{AnalysisConfig, Config},
    contracts::ContractConfig,
    error::{Alternatives, ConfigError, ConfigErrorKind, InContract, PatternError, classify},
    language::{Language, Languages},
    parts::{PartGlob, PartGlobs},
    schema::{ConfigFile, Located, WorkspaceTable},
};

/// A value of the TOML document, with its byte range in the file.
type TomlValue<'de> = Spanned<DeValue<'de>>;

/// Parse and validate the contents of a configuration file, as
/// [`Config::parse`] does.
pub(crate) fn parse(contents: &str, languages: Languages) -> Result<Config, Vec<ConfigError>> {
    let (parsed, errors) = parse_root(contents, languages);
    match parsed {
        Some(root) if errors.is_empty() => Ok(root.config),
        _ => Err(errors),
    }
}

/// The root's configuration file, parsed.
#[derive(Debug)]
pub(crate) struct ParsedRoot {
    /// Its configuration.
    pub(crate) config: Config,

    /// The globs of the files it is split over.
    pub(crate) globs: PartGlobs,

    /// Its languages, which the files that are part of it are parsed with.
    pub(crate) languages: RootLanguages,
}

/// The languages of the root's configuration, whose selector schemes the
/// contracts of the files that are part of it may use.
#[derive(Debug, Clone)]
pub(crate) struct RootLanguages {
    /// The enabled languages.
    enabled: Vec<&'static Language>,

    /// Whether its `[languages]` says which languages are enabled. If it
    /// names an unknown language, or isn't a table, the enabled ones aren't
    /// those meant, so the schemes of the others aren't errors.
    known: bool,
}

/// Parse and validate the contents of the root's configuration file, with
/// the globs of the files it is split over. Returns it, or `None` if it isn't
/// valid TOML, and the errors, in the order of the file.
pub(crate) fn parse_root(
    contents: &str,
    languages: Languages,
) -> (Option<ParsedRoot>, Vec<ConfigError>) {
    let mut parser = Parser::new(languages, Vec::new());
    let parsed = parser.parse_config(contents).map(|(config, globs)| {
        let languages =
            RootLanguages { enabled: parser.enabled.clone(), known: parser.languages_known };
        ParsedRoot { config, globs, languages }
    });
    (parsed, parser.into_errors())
}

/// Parse and validate the contents of a file that is part of the root's
/// configuration, whose languages are `root`: one that it includes, or the
/// file of the `member` in that directory, or one that the member includes.
/// Its position in
/// [`Config::config_files`] is `file`, and its `[analysis]` defaults to
/// `analysis`, that of the file that includes it, or the root's. Returns what
/// it adds to the root's, with the globs of the files it includes, or `None`
/// if it isn't valid TOML, and the errors, in the order of the file.
pub(crate) fn parse_part(
    contents: &str,
    root: &RootLanguages,
    analysis: &AnalysisConfig,
    file: usize,
    member: Option<&str>,
    languages: Languages,
) -> (Option<(PartConfig, PartGlobs)>, Vec<ConfigError>) {
    let mut parser = Parser::new(languages, root.enabled.clone());
    parser.languages_known = root.known;
    (parser.file, parser.member) = (file, member.map(String::from));
    let part = parser.parse_part(contents, analysis);
    (part, parser.into_errors())
}

/// Whether the contents of a configuration file list the directory
/// `member`, a POSIX path relative to its own, in `[workspace] members`. Only
/// that table is read, so a file with errors elsewhere still lists its
/// members.
pub(crate) fn lists_member(contents: &str, member: &str) -> bool {
    /// A configuration file, with only `[workspace]` read.
    #[derive(Deserialize)]
    struct Members {
        /// Its `[workspace]`.
        #[serde(default)]
        workspace: WorkspaceTable,
    }

    let Ok(Members { workspace }) = toml::from_str::<Members>(contents) else {
        return false;
    };
    let is_match = |pattern: &str| Glob::new(pattern).is_ok_and(|glob| glob.is_match(member));
    workspace.members.iter().any(|pattern| is_match(&pattern.value))
}

/// What a file that is part of the root's configuration adds to it.
#[derive(Debug)]
pub(crate) struct PartConfig {
    /// The globs of files and directories to skip, relative to the root.
    pub(crate) exclude: Vec<Glob>,

    /// The contracts, in the order of the file.
    pub(crate) contracts: Vec<ContractConfig>,

    /// Its `[analysis]`, which the files it includes default to.
    pub(crate) analysis: AnalysisConfig,
}

/// The tables that only the root's configuration file may have.
const ROOT_TABLES: [&str; 4] = ["workspace", "languages", "baseline", "cache"];

/// The keys that a file that is part of the root's configuration may have.
const PART_TABLES: [&str; 4] = ["include", "files", "analysis", "contracts"];

/// Parse and validate the contracts in `contents` as contracts of `config`,
/// as [`Config::parse_contracts`] does.
pub(crate) fn parse_contracts(
    contents: &str,
    config: &Config,
    languages: Languages,
) -> Result<Vec<ContractConfig>, Vec<ConfigError>> {
    let mut parser = Parser::new(languages, config.languages.enabled().collect());
    let contracts = parser.parse_document(contents).map(|mut document| {
        // Only `[contracts]` may be written; the other tables are the configuration's.
        let contracts = document.get_mut().remove("contracts");
        for key in document.get_ref().keys() {
            let alternatives = Alternatives::new(key.get_ref(), vec!["contracts".to_string()]);
            let kind = ConfigErrorKind::UnknownKey { key: key.get_ref().to_string(), alternatives };
            parser.add_error(kind, Some(ByteRange::from(key.span())));
        }
        match contracts {
            Some(contracts) => parser.parse_contracts(contracts, &config.analysis),
            None => Vec::new(),
        }
    });
    parser.finish(contracts)
}

/// Parses a configuration file, collecting its errors.
struct Parser {
    /// The errors found so far.
    diagnostics: DiagnosticStore<ConfigError>,

    /// The id of the contract being parsed, which errors are attributed to.
    contract: Option<InContract>,

    /// The languages that the file may enable.
    languages: Languages,

    /// The enabled languages, whose selector schemes the contracts may use.
    /// Known once `[languages]` is parsed.
    enabled: Vec<&'static Language>,

    /// Whether `[languages]` says which languages are enabled, as
    /// [`RootLanguages`] has it. If not, a scheme of a language that isn't
    /// enabled isn't an error.
    languages_known: bool,

    /// The position of the file in [`Config::config_files`], which its
    /// contracts record.
    file: usize,

    /// The directory of the member whose file this is, relative to the root,
    /// which its paths are relative to. `None` for the root's file, and the
    /// files it includes.
    member: Option<String>,
}

impl Parser {
    /// A parser of the root's file, which may enable the `languages`, whose
    /// contracts may use the selector schemes of the `enabled` ones.
    fn new(languages: Languages, enabled: Vec<&'static Language>) -> Self {
        Self {
            diagnostics: DiagnosticStore::default(),
            contract: None,
            languages,
            enabled,
            languages_known: true,
            file: 0,
            member: None,
        }
    }

    /// `parsed`, or else the errors found, in the order of the file.
    fn finish<T>(self, parsed: Option<T>) -> Result<T, Vec<ConfigError>> {
        let errors = self.into_errors();
        match parsed {
            Some(parsed) if errors.is_empty() => Ok(parsed),
            _ => Err(errors),
        }
    }

    /// The errors found, in the order of the file.
    fn into_errors(self) -> Vec<ConfigError> {
        let (mut errors, _) = self.diagnostics.into_parts();
        ConfigError::sort(&mut errors);
        errors
    }

    /// Parse the TOML document, which keeps the byte range of every key and
    /// value. Returns `None` if it isn't valid TOML.
    fn parse_document<'de>(&mut self, contents: &'de str) -> Option<Spanned<DeTable<'de>>> {
        match DeTable::parse(contents) {
            Ok(document) => Some(document),
            Err(error) => {
                let kind = ConfigErrorKind::Syntax { message: error.message().to_string() };
                self.add_error(kind, error.span().map(ByteRange::from));
                None
            }
        }
    }

    /// Parse the whole of the root's file, and the globs of the files it is
    /// split over. Returns `None` if it isn't valid TOML.
    fn parse_config(&mut self, contents: &str) -> Option<(Config, PartGlobs)> {
        // 1. Parse the TOML document.
        let mut document = self.parse_document(contents)?;

        // 2. Take out the known tables, and reject the ones left, which are unknown.
        let tables = document.get_mut();
        let include = tables.remove("include");
        let workspace = tables.remove("workspace");
        let files = tables.remove("files");
        let languages = tables.remove("languages");
        let analysis = tables.remove("analysis");
        let baseline = tables.remove("baseline");
        let cache = tables.remove("cache");
        let contracts = tables.remove("contracts");
        if let Err(error) = ConfigFile::deserialize(Deserializer::from(document)) {
            self.add_toml_error(&error);
        }

        // 3. Parse each global table on its own. One that isn't written, or is invalid,
        //    has its defaults, so that the rest of the file is still checked.
        let include = self.parse_table(include).unwrap_or_default();
        let members = self.parse_table::<WorkspaceTable>(workspace).unwrap_or_default().members;
        let parts = PartGlobs {
            include: self.parse_part_globs(include),
            members: self.parse_part_globs(members),
        };
        let files = self.parse_files(files);
        let languages = self.parse_languages(languages);
        self.enabled = languages.enabled().collect();
        let analysis = self.parse_analysis(analysis, &AnalysisConfig::default());
        let baseline = self.parse_baseline(baseline);
        let cache = self.parse_cache(cache);

        // 4. Parse the contracts, which default to the options in `[analysis]`.
        let contracts = match contracts {
            Some(contracts) => self.parse_contracts(contracts, &analysis),
            None => Vec::new(),
        };

        let config = Config {
            files,
            languages,
            analysis,
            baseline,
            cache,
            contracts,
            config_files: Vec::new(),
        };
        Some((config, parts))
    }

    /// Parse a file that is part of the root's configuration, whose
    /// `[analysis]` defaults to `defaults`, and the globs of the files it
    /// includes. Returns `None` if it isn't valid TOML.
    fn parse_part(
        &mut self,
        contents: &str,
        defaults: &AnalysisConfig,
    ) -> Option<(PartConfig, PartGlobs)> {
        // 1. Parse the TOML document.
        let mut document = self.parse_document(contents)?;

        // 2. Take out the tables it may have, and reject the ones left: the root's, and
        //    the unknown ones.
        let tables = document.get_mut();
        let include = tables.remove("include");
        let files = tables.remove("files");
        let analysis = tables.remove("analysis");
        let contracts = tables.remove("contracts");
        for key in tables.keys() {
            let key_name = key.get_ref().to_string();
            let kind = match ROOT_TABLES.contains(&key_name.as_str()) {
                true => ConfigErrorKind::RootOnly { key: key_name },
                false => {
                    let expected = PART_TABLES.map(String::from).to_vec();
                    let alternatives = Alternatives::new(&key_name, expected);
                    ConfigErrorKind::UnknownKey { key: key_name, alternatives }
                }
            };
            self.add_error(kind, Some(ByteRange::from(key.span())));
        }

        // 3. Parse its tables, with the options of the file that includes it, or the
        //    root's, as their defaults.
        let include = self.parse_table(include).unwrap_or_default();
        let globs = PartGlobs { include: self.parse_part_globs(include), members: Vec::new() };
        let exclude = self.parse_part_files(files);
        let analysis = self.parse_analysis(analysis, defaults);
        let contracts = match contracts {
            Some(contracts) => self.parse_contracts(contracts, &analysis),
            None => Vec::new(),
        };
        Some((PartConfig { exclude, contracts, analysis }, globs))
    }

    /// Compile the globs of `include` or `[workspace] members`, leaving out
    /// the invalid ones, which include those with a `..` segment. A member's
    /// are under its directory.
    fn parse_part_globs(&mut self, patterns: Vec<Located<String>>) -> Vec<PartGlob> {
        patterns
            .into_iter()
            .filter_map(|Located { value: pattern, range }| {
                if pattern.split('/').any(|segment| segment == "..") {
                    self.add_error(ConfigErrorKind::PatternParentDir { pattern }, Some(range));
                    return None;
                }
                let glob = self.check(Glob::new(&pattern), Some(range), |error| {
                    ConfigErrorKind::InvalidPattern {
                        pattern: pattern.clone(),
                        error: PatternError(error),
                    }
                })?;
                let glob = match &self.member {
                    Some(member) => glob.under(member),
                    None => glob,
                };
                Some(PartGlob { pattern, glob, range })
            })
            .collect()
    }

    /// Compile globs or name patterns, leaving out the invalid ones.
    fn parse_patterns<T>(
        &mut self,
        patterns: &[Located<String>],
        compile: impl Fn(&str) -> Result<T, GlobError>,
    ) -> Vec<T> {
        patterns
            .iter()
            .filter_map(|pattern| {
                self.check(compile(&pattern.value), Some(pattern.range), |error| {
                    ConfigErrorKind::InvalidPattern {
                        pattern: pattern.value.clone(),
                        error: PatternError(error),
                    }
                })
            })
            .collect()
    }

    /// Record an error at a byte range of the file.
    fn add_error(&mut self, kind: ConfigErrorKind, range: Option<ByteRange>) {
        let error = self.error(kind, range);
        self.diagnostics.add_error(error);
    }

    /// Record an error of the TOML deserialiser.
    fn add_toml_error(&mut self, error: &toml::de::Error) {
        self.add_error(classify(error.message()), error.span().map(ByteRange::from));
    }

    /// The value of `result`, or else record its error at `range`.
    fn check<T, E>(
        &mut self,
        result: Result<T, E>,
        range: Option<ByteRange>,
        kind: impl FnOnce(E) -> ConfigErrorKind,
    ) -> Option<T> {
        let result = result.map_err(|error| self.error(kind(error), range));
        self.diagnostics.try_or_add_error(result)
    }

    /// An error at a byte range of the file, attributed to the contract being
    /// parsed.
    fn error(&self, kind: ConfigErrorKind, range: Option<ByteRange>) -> ConfigError {
        ConfigError { kind, contract: self.contract.clone(), range }
    }

    /// Run `parse`, attributing its errors to the contract `id`.
    fn in_contract<T>(&mut self, id: &str, parse: impl FnOnce(&mut Self) -> T) -> T {
        self.contract = Some(InContract(id.to_string()));
        let value = parse(self);
        self.contract = None;
        value
    }
}
