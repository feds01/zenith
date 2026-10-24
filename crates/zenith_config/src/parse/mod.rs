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
use zenith_utils::{glob::GlobError, location::ByteRange};

use crate::{
    config::Config,
    contracts::ContractConfig,
    error::{Alternatives, ConfigError, ConfigErrorKind, InContract, PatternError, classify},
    language::{Language, Languages},
    schema::{ConfigFile, Located},
};

/// A value of the TOML document, with its byte range in the file.
type TomlValue<'de> = Spanned<DeValue<'de>>;

/// Parse and validate the contents of a configuration file, as
/// [`Config::parse`] does.
pub(crate) fn parse(contents: &str, languages: Languages) -> Result<Config, Vec<ConfigError>> {
    let mut parser = Parser::new(languages, Vec::new());
    let config = parser.parse_config(contents);
    parser.finish(config)
}

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
}

impl Parser {
    /// A parser of a file that may enable the `languages`, whose contracts may
    /// use the selector schemes of the `enabled` ones.
    fn new(languages: Languages, enabled: Vec<&'static Language>) -> Self {
        Self { diagnostics: DiagnosticStore::default(), contract: None, languages, enabled }
    }

    /// `parsed`, or else the errors found, in the order of the file.
    fn finish<T>(self, parsed: Option<T>) -> Result<T, Vec<ConfigError>> {
        let (mut errors, _) = self.diagnostics.into_parts();
        match parsed {
            Some(parsed) if errors.is_empty() => Ok(parsed),
            _ => {
                errors.sort_by_key(|error| error.range.map(ByteRange::start));
                Err(errors)
            }
        }
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

    /// Parse the whole file. Returns `None` if it isn't valid TOML.
    fn parse_config(&mut self, contents: &str) -> Option<Config> {
        // 1. Parse the TOML document.
        let mut document = self.parse_document(contents)?;

        // 2. Take out the known tables, and reject the ones left, which are unknown.
        let tables = document.get_mut();
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
        let files = self.parse_files(files);
        let languages = self.parse_languages(languages);
        self.enabled = languages.enabled().collect();
        let analysis = self.parse_analysis(analysis);
        let baseline = self.parse_baseline(baseline);
        let cache = self.parse_cache(cache);

        // 4. Parse the contracts, which default to the options in `[analysis]`.
        let contracts = match contracts {
            Some(contracts) => self.parse_contracts(contracts, &analysis),
            None => Vec::new(),
        };

        Some(Config { files, languages, analysis, baseline, cache, contracts, file: None })
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
