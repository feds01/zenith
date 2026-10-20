//! Parsing `[languages]`: which languages are enabled, and the options of
//! each.
//!
//! The table of each language is deserialised on its own, so that its errors
//! point at the offending key, and the names that a language's table goes by
//! are looked up in the registry, so that giving one language twice, under
//! two of its names, is an error.

use toml::de::DeValue;
use zenith_utils::{
    glob::{Glob, NamePattern},
    location::ByteRange,
};

use super::{Parser, TomlValue};
use crate::{
    config::{JsConfig, LanguagesConfig, Tsconfig},
    error::{ConfigErrorKind, suggest},
    language::{LANGUAGES, Language, LanguageId},
    schema::JsTable,
};

impl Parser {
    /// Parse `[languages]`. Without it, JavaScript and TypeScript are enabled
    /// with their defaults. With it, only the languages that it has a table
    /// for are, and there must be one.
    pub(super) fn parse_languages(&mut self, value: Option<TomlValue<'_>>) -> LanguagesConfig {
        // 1. Without the table, the defaults.
        let Some(value) = value else {
            return LanguagesConfig::default();
        };
        let range = ByteRange::from(value.span());
        let DeValue::Table(table) = value.into_inner() else {
            let kind = ConfigErrorKind::NotATable { key: "languages".to_string() };
            self.add_error(kind, Some(range));
            return LanguagesConfig::default();
        };
        if table.is_empty() {
            self.add_error(ConfigErrorKind::NoLanguages, Some(range));
        }

        // 2. Parse the table of each language, in the order of the file, so that the
        //    second table of a language is the one reported.
        let mut tables: Vec<_> = table.into_iter().collect();
        tables.sort_by_key(|(name, _)| name.span().start);
        let mut languages = LanguagesConfig { js: None };
        let mut seen: Vec<(&'static Language, String)> = Vec::new();
        for (name, value) in tables {
            let name_range = ByteRange::from(name.span());
            let name = name.into_inner();
            let Some(language) = Language::by_table_name(&name) else {
                let names: Vec<_> = LANGUAGES
                    .iter()
                    .flat_map(|language| language.table_names)
                    .map(|name| name.to_string())
                    .collect();
                let kind = ConfigErrorKind::UnknownLanguage {
                    name: name.to_string(),
                    suggestion: suggest(&name, names.iter().map(String::as_str)).map(String::from),
                    expected: names,
                };
                self.add_error(kind, Some(name_range));
                continue;
            };
            if let Some((_, first)) = seen.iter().find(|(seen, _)| seen.id == language.id) {
                let kind = ConfigErrorKind::DuplicateLanguage {
                    name: name.to_string(),
                    first: first.clone(),
                };
                self.add_error(kind, Some(name_range));
                continue;
            }
            seen.push((language, name.to_string()));
            match language.id {
                LanguageId::Js => languages.js = Some(self.parse_js(value)),
            }
        }
        languages
    }

    /// Parse the table of JavaScript and TypeScript. An invalid table has
    /// the defaults, so that the language is still enabled.
    fn parse_js(&mut self, value: TomlValue<'_>) -> JsConfig {
        let Some(table) = self.parse_table::<JsTable>(Some(value)) else {
            return JsConfig::default();
        };
        let tsconfig = match table.tsconfig.as_deref() {
            None | Some("auto") => Tsconfig::Auto,
            Some("off") => Tsconfig::Off,
            Some(path) => Tsconfig::File(path.to_string()),
        };
        JsConfig {
            include: table.include.map(|include| self.parse_patterns(&include, Glob::new)),
            tsconfig,
            conditions: table.conditions,
            main_fields: table.main_fields,
            extensions: table.extensions,
            alias: table.alias,
            ignore_unresolved: self.parse_patterns(&table.ignore_unresolved, NamePattern::new),
        }
    }
}
