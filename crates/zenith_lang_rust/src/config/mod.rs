//! Rust in the configuration: the [`RUST`] language, the `[languages.rust]`
//! table, and the options it is parsed into.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_config::{Language, LanguageOptions, LanguageTable, Located};
use zenith_lang::{Backend, BackendSettings};
use zenith_utils::glob::Glob;

use crate::{backend::RustBackend, crates::Discovery};

/// Rust.
pub const RUST: Language = Language {
    name: "rust",
    table_names: &["rust"],
    description: "Rust: how its files are found and the imports between its crates and modules \
                  resolved.",
    enabled_by_default: false,
    include: &["**/*.rs"],
    source_extensions: &["rs"],
    optional_extensions: &["rs"],
    // `crate:` is for external crates, `rust:` for those of the standard library.
    schemes: &["crate", "rust"],
    parse: |table| Arc::new(RustConfig::parse(table)),
    defaults: || Arc::new(RustConfig::default()),
    schema: |generator| generator.subschema_for::<RustTable>(),
};

/// The options of Rust. `None` options leave the defaults in place.
#[derive(Debug, Clone, Default)]
pub struct RustConfig {
    /// Globs of the files to scan. `None` scans every file with one of the
    /// [`RUST`] source extensions.
    pub include: Option<Vec<Glob>>,
}

impl RustConfig {
    /// Parse the table of Rust. An invalid table has the defaults, so that
    /// the language is still enabled.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<RustTable>() else {
            return Self::default();
        };
        Self { include: parsed.include.map(|include| table.patterns(&include, Glob::new)) }
    }
}

impl LanguageOptions for RustConfig {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }

    fn backend(&self, settings: BackendSettings<'_>) -> Box<dyn Backend> {
        Box::new(RustBackend::new(settings.root, Discovery::Cargo))
    }
}

/// Rust.
// The table as written, which is its schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
struct RustTable {
    /// Globs of the files to scan. Defaults to every Rust file, `**/*.rs`.
    include: Option<Vec<Located<String>>>,
}
