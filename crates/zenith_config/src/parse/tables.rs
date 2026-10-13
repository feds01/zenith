//! Parsing the global tables: `[files]`, `[resolve]`, `[analysis]`,
//! `[baseline]` and `[cache]`.

use serde::Deserialize;
use toml::de::ValueDeserializer;
use zenith_utils::glob::{Glob, NamePattern};

use super::{Parser, TomlValue};
use crate::{
    config::{AnalysisConfig, BaselineConfig, CacheConfig, FilesConfig, ResolveConfig, Tsconfig},
    schema::{AnalysisTable, BaselineTable, CacheTable, FilesTable, ResolveTable},
};

impl Parser {
    /// Parse `[files]`.
    pub(super) fn parse_files(&mut self, value: Option<TomlValue<'_>>) -> FilesConfig {
        let Some(table) = self.parse_table::<FilesTable>(value) else {
            return FilesConfig::default();
        };
        FilesConfig {
            include: table.include.map(|include| self.parse_patterns(&include, Glob::new)),
            exclude: self.parse_patterns(&table.exclude, Glob::new),
            respect_gitignore: table.respect_gitignore,
        }
    }

    /// Parse `[resolve]`.
    pub(super) fn parse_resolve(&mut self, value: Option<TomlValue<'_>>) -> ResolveConfig {
        let Some(table) = self.parse_table::<ResolveTable>(value) else {
            return ResolveConfig::default();
        };
        let tsconfig = match table.tsconfig.as_deref() {
            None | Some("auto") => Tsconfig::Auto,
            Some("off") => Tsconfig::Off,
            Some(path) => Tsconfig::File(path.to_string()),
        };
        ResolveConfig {
            tsconfig,
            conditions: table.conditions,
            main_fields: table.main_fields,
            extensions: table.extensions,
            alias: table.alias,
            ignore_unresolved: self.parse_patterns(&table.ignore_unresolved, NamePattern::new),
        }
    }

    /// Parse `[analysis]`.
    pub(super) fn parse_analysis(&mut self, value: Option<TomlValue<'_>>) -> AnalysisConfig {
        let Some(table) = self.parse_table::<AnalysisTable>(value) else {
            return AnalysisConfig::default();
        };
        AnalysisConfig {
            exclude_kinds: table.exclude_kinds.into_iter().collect(),
            reexports: table.reexports,
        }
    }

    /// Parse `[baseline]`.
    pub(super) fn parse_baseline(&mut self, value: Option<TomlValue<'_>>) -> BaselineConfig {
        let Some(table) = self.parse_table::<BaselineTable>(value) else {
            return BaselineConfig::default();
        };
        BaselineConfig { path: table.path, stale: table.stale }
    }

    /// Parse `[cache]`.
    pub(super) fn parse_cache(&mut self, value: Option<TomlValue<'_>>) -> CacheConfig {
        let Some(table) = self.parse_table::<CacheTable>(value) else {
            return CacheConfig::default();
        };
        CacheConfig { enabled: table.enabled, dir: table.dir }
    }

    /// Deserialise a global table, or its defaults when it isn't written.
    /// Returns `None` if it is invalid.
    fn parse_table<'de, T: Deserialize<'de> + Default>(
        &mut self,
        value: Option<TomlValue<'de>>,
    ) -> Option<T> {
        let Some(value) = value else {
            return Some(T::default());
        };
        T::deserialize(ValueDeserializer::from(value))
            .inspect_err(|error| self.add_toml_error(error))
            .ok()
    }
}
