//! Parsing `[languages]`: which languages are enabled, and the options of
//! each.
//!
//! Each language parses its own table, through a [`LanguageTable`], so that
//! its errors point at the offending key, and the names that a language's
//! table goes by are looked up in the registry, so that giving one language
//! twice, under two of its names, is an error.

use std::sync::Arc;

use serde::Deserialize;
use toml::de::{DeValue, ValueDeserializer};
use zenith_utils::{glob::GlobError, location::ByteRange};

use super::{Parser, TomlValue};
use crate::{
    config::LanguagesConfig,
    error::{Alternatives, ConfigErrorKind},
    language::{Language, LanguageOptions},
    schema::Located,
};

impl Parser {
    /// Parse `[languages]`. Without it, the languages that are enabled by
    /// default are, with their defaults. With it, only the languages that it
    /// has a table for are, and there must be one.
    pub(super) fn parse_languages(&mut self, value: Option<TomlValue<'_>>) -> LanguagesConfig {
        // 1. Without the table, the defaults.
        let Some(value) = value else {
            return LanguagesConfig::defaults(self.languages);
        };
        let range = ByteRange::from(value.span());
        let DeValue::Table(table) = value.into_inner() else {
            let kind = ConfigErrorKind::NotATable { key: "languages".to_string() };
            self.add_error(kind, Some(range));
            return LanguagesConfig::defaults(self.languages);
        };
        if table.is_empty() {
            let example = self.languages.iter().next().map(|language| language.name.to_string());
            self.add_error(ConfigErrorKind::NoLanguages { example }, Some(range));
        }

        // 2. Parse the table of each language, in the order of the file, so that the
        //    second table of a language is the one reported.
        let mut tables: Vec<_> = table.into_iter().collect();
        tables.sort_by_key(|(name, _)| name.span().start);
        let mut enabled: Vec<(&'static Language, String, Arc<dyn LanguageOptions>)> = Vec::new();
        for (name, value) in tables {
            let name_range = ByteRange::from(name.span());
            let name = name.into_inner();
            let Some(language) = self.languages.by_table_name(&name) else {
                let names: Vec<_> = self.languages.table_names().map(String::from).collect();
                let kind = ConfigErrorKind::UnknownLanguage {
                    name: name.to_string(),
                    alternatives: Alternatives::new(&name, names),
                };
                self.add_error(kind, Some(name_range));
                continue;
            };
            if let Some((_, first, _)) = enabled.iter().find(|(seen, ..)| *seen == language) {
                let kind = ConfigErrorKind::DuplicateLanguage {
                    name: name.to_string(),
                    first: first.clone(),
                };
                self.add_error(kind, Some(name_range));
                continue;
            }
            let options = (language.parse)(&mut LanguageTable { parser: self, value: Some(value) });
            enabled.push((language, name.to_string(), options));
        }

        // 3. Keep them in the order of the registry, which the walk and the docs use.
        enabled.sort_by_key(|(language, ..)| {
            self.languages.iter().position(|known| known == *language)
        });
        LanguagesConfig::new(
            enabled.into_iter().map(|(language, _, options)| (language, options)).collect(),
        )
    }
}

/// A language's table in `[languages]`, being parsed. Its errors are
/// collected with those of the rest of the file.
pub struct LanguageTable<'p, 'de> {
    /// The parser of the file.
    parser: &'p mut Parser,

    /// The table, until it is deserialised.
    value: Option<TomlValue<'de>>,
}

impl<'de> LanguageTable<'_, 'de> {
    /// Deserialise the table as `T`. Returns `None` if it is invalid, after
    /// recording its first error, or if it was deserialised already.
    pub fn deserialize<T: Deserialize<'de>>(&mut self) -> Option<T> {
        let value = self.value.take()?;
        T::deserialize(ValueDeserializer::from(value))
            .inspect_err(|error| self.parser.add_toml_error(error))
            .ok()
    }

    /// Compile globs or name patterns, leaving out the invalid ones after
    /// recording their errors.
    pub fn patterns<T>(
        &mut self,
        patterns: &[Located<String>],
        compile: impl Fn(&str) -> Result<T, GlobError>,
    ) -> Vec<T> {
        self.parser.parse_patterns(patterns, compile)
    }
}
