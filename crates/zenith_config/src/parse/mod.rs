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
//! `contracts` the contracts, and `selector` and `layer` the strings that
//! selectors, ignored imports and layers are written as.

mod contracts;
mod layer;
mod selector;
mod tables;
#[cfg(test)]
mod tests;

use serde::Deserialize;
use toml::{
    Spanned,
    de::{DeTable, DeValue, Deserializer},
};
use zenith_diagnostics::DiagnosticStore;
use zenith_utils::{glob::GlobError, location::ByteRange};

use crate::{
    config::Config,
    error::{ConfigError, ConfigErrorKind, classify},
    schema::{ConfigFile, Located},
};

/// A value of the TOML document, with its byte range in the file.
type TomlValue<'de> = Spanned<DeValue<'de>>;

/// Parse and validate the contents of a configuration file, as
/// [`Config::parse`] does.
pub(crate) fn parse(contents: &str) -> Result<Config, Vec<ConfigError>> {
    let mut parser = Parser::default();
    let config = parser.parse_config(contents);
    let (mut errors, _) = parser.diagnostics.into_parts();
    match config {
        Some(config) if errors.is_empty() => Ok(config),
        _ => {
            errors.sort_by_key(|error| error.range.map(ByteRange::start));
            Err(errors)
        }
    }
}

/// Parses a configuration file, collecting its errors.
#[derive(Default)]
struct Parser {
    /// The errors found so far.
    diagnostics: DiagnosticStore<ConfigError>,

    /// The id of the contract being parsed, which errors are attributed to.
    contract: Option<String>,
}

impl Parser {
    /// Parse the whole file. Returns `None` if it isn't valid TOML.
    fn parse_config(&mut self, contents: &str) -> Option<Config> {
        // 1. Parse the TOML document, which keeps the byte range of every key and
        //    value.
        let mut document = match DeTable::parse(contents) {
            Ok(document) => document,
            Err(error) => {
                let kind = ConfigErrorKind::Syntax { message: error.message().to_string() };
                self.add_error(kind, error.span().map(ByteRange::from));
                return None;
            }
        };

        // 2. Take out the known tables, and reject the ones left, which are unknown.
        let tables = document.get_mut();
        let files = tables.remove("files");
        let resolve = tables.remove("resolve");
        let analysis = tables.remove("analysis");
        let baseline = tables.remove("baseline");
        let contracts = tables.remove("contracts");
        if let Err(error) = ConfigFile::deserialize(Deserializer::from(document)) {
            self.add_toml_error(&error);
        }

        // 3. Parse each global table on its own. One that isn't written, or is invalid,
        //    has its defaults, so that the rest of the file is still checked.
        let files = self.parse_files(files);
        let resolve = self.parse_resolve(resolve);
        let analysis = self.parse_analysis(analysis);
        let baseline = self.parse_baseline(baseline);

        // 4. Parse the contracts, which default to the options in `[analysis]`.
        let contracts = match contracts {
            Some(contracts) => self.parse_contracts(contracts, &analysis),
            None => Vec::new(),
        };

        Some(Config { files, resolve, analysis, baseline, contracts })
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
                    ConfigErrorKind::InvalidPattern { pattern: pattern.value.clone(), error }
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
        self.contract = Some(id.to_string());
        let value = parse(self);
        self.contract = None;
        value
    }
}
