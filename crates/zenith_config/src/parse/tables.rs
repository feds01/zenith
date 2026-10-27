//! Parsing the global tables: `[files]`, `[analysis]`, `[baseline]` and
//! `[cache]`. `[languages]` has a module of its own.
//!
//! A file that the root includes, or a member's, may only have the `exclude`
//! of `[files]`, and `[analysis]`, whose options default to the root's.

use serde::Deserialize;
use toml::{
    Spanned,
    de::{DeValue, ValueDeserializer},
};
use zenith_utils::{glob::Glob, location::ByteRange};

use super::{Parser, TomlValue};
use crate::{
    config::{AnalysisConfig, BaselineConfig, CacheConfig, FilesConfig},
    error::ConfigErrorKind,
    schema::{AnalysisTable, BaselineTable, CacheTable, FilesTable},
};

impl Parser {
    /// Parse `[files]`.
    pub(super) fn parse_files(&mut self, value: Option<TomlValue<'_>>) -> FilesConfig {
        let Some(table) = self.parse_table::<FilesTable>(value) else {
            return FilesConfig::default();
        };
        let member = self.member.clone();
        let exclude = self.parse_patterns(&table.exclude, |pattern| {
            let glob = Glob::new(pattern)?;
            Ok(match &member {
                Some(member) => glob.under(member),
                None => glob,
            })
        });
        FilesConfig { exclude, respect_gitignore: table.respect_gitignore }
    }

    /// Parse `[files]` in a file that the root includes, or in a member's,
    /// which may only have its own `exclude`.
    pub(super) fn parse_part_files(&mut self, mut value: Option<TomlValue<'_>>) -> Vec<Glob> {
        if let Some(DeValue::Table(table)) = value.as_mut().map(Spanned::get_mut)
            && let Some((key, _)) = table.remove_entry("respect_gitignore")
        {
            let kind = ConfigErrorKind::RootOnly { key: "files.respect_gitignore".to_string() };
            self.add_error(kind, Some(ByteRange::from(key.span())));
        }
        self.parse_files(value).exclude
    }

    /// Parse `[analysis]`, whose options default to those of `defaults`.
    pub(super) fn parse_analysis(
        &mut self,
        value: Option<TomlValue<'_>>,
        defaults: &AnalysisConfig,
    ) -> AnalysisConfig {
        let written = |key: &str| matches!(value.as_ref().map(Spanned::get_ref), Some(DeValue::Table(table)) if table.contains_key(key));
        let (exclude_kinds, reexports) = (written("exclude_kinds"), written("reexports"));
        let Some(table) = self.parse_table::<AnalysisTable>(value) else {
            return defaults.clone();
        };
        AnalysisConfig {
            exclude_kinds: match exclude_kinds {
                true => table.exclude_kinds.into_iter().collect(),
                false => defaults.exclude_kinds,
            },
            reexports: if reexports { table.reexports } else { defaults.reexports },
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
    pub(super) fn parse_table<'de, T: Deserialize<'de> + Default>(
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
