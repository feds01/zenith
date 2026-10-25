//! Rust in the configuration: the [`RUST`] language, the `[languages.rust]`
//! table, and the options it is parsed into.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_config::{Language, LanguageOptions, LanguageTable, Located};
use zenith_lang::{Backend, BackendSettings};
use zenith_utils::glob::{Glob, NamePattern};

use crate::backend::RustBackend;

/// Rust.
pub const RUST: Language = Language {
    name: "rust",
    table_names: &["rust"],
    description: "Rust: how its files are found and the imports between its crates and modules \
                  resolved.",
    enabled_by_default: false,
    include: &["**/*.rs"],
    source_extensions: &["rs"],
    source_file_names: &[],
    optional_extensions: &["rs"],
    index_files: &["lib.rs", "main.rs", "mod.rs"],
    // `crate:` is for external crates, `rust:` for those of the standard library.
    schemes: &["crate", "rust"],
    // The paths into the standard library, such as `rust:std::thread`, nest.
    path_schemes: &[("rust", "::")],
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

    /// Patterns of specifiers that are expected not to resolve, so are not
    /// reported as unresolved.
    pub ignore_unresolved: Vec<NamePattern>,
}

impl RustConfig {
    /// Parse the table of Rust. An invalid table has the defaults, so that
    /// the language is still enabled.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<RustTable>() else {
            return Self::default();
        };
        Self {
            include: parsed.include.map(|include| table.patterns(&include, Glob::new)),
            ignore_unresolved: table.patterns(&parsed.ignore_unresolved, NamePattern::new),
        }
    }
}

impl LanguageOptions for RustConfig {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }

    fn backend(&self, settings: BackendSettings<'_>) -> Box<dyn Backend> {
        let (file_system, ignore_unresolved) =
            (settings.file_system.clone(), self.ignore_unresolved.clone());
        Box::new(RustBackend::new(
            settings.root,
            file_system,
            ignore_unresolved,
            settings.record_exports,
        ))
    }
}

/// Rust.
// The table as written, which is its schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
struct RustTable {
    /// Globs of the files to scan. Defaults to every Rust file, `**/*.rs`.
    include: Option<Vec<Located<String>>>,

    /// Name patterns of paths that are expected not to resolve, so are not
    /// reported as unresolved, e.g. `"generated::*"` for a module that a
    /// macro declares.
    ignore_unresolved: Vec<Located<String>>,
}
