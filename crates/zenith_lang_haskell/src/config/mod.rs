//! Haskell in the configuration: the [`HASKELL`] language, the
//! `[languages.haskell]` table, and the options it is parsed into.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_config::{Language, LanguageOptions, LanguageTable, Located};
use zenith_lang::{Backend, BackendSettings};
use zenith_utils::glob::{Glob, NamePattern};

use crate::backend::HaskellBackend;

/// Haskell.
pub const HASKELL: Language = Language {
    name: "haskell",
    table_names: &["haskell"],
    description: "Haskell: how its files are found and the imports between its modules resolved.",
    enabled_by_default: false,
    include: &[
        "**/*.hs",
        "**/*.lhs",
        "**/*.hs-boot",
        "**/*.lhs-boot",
        "**/*.hsig",
        "**/*.lhsig",
        "**/*.hsc",
    ],
    source_extensions: &["hs", "lhs", "hs-boot", "lhs-boot", "hsig", "lhsig", "hsc"],
    source_file_names: &[],
    optional_extensions: &["hs", "lhs", "hs-boot", "lhs-boot", "hsig", "lhsig", "hsc"],
    directory_modules: &[],
    index_files: &[],
    // `hackage:` is for the packages that a build installed, `base` among them.
    schemes: &["hackage"],
    path_schemes: &[],
    parse: |table| Arc::new(HaskellConfig::parse(table)),
    defaults: || Arc::new(HaskellConfig::default()),
    schema: |generator| generator.subschema_for::<HaskellTable>(),
};

/// The options of Haskell. `None` options leave the defaults in place.
#[derive(Debug, Clone, Default)]
pub struct HaskellConfig {
    /// Globs of the files to scan. `None` scans every file with one of the
    /// [`HASKELL`] source extensions.
    pub include: Option<Vec<Glob>>,

    /// Patterns of the specifiers that are expected not to resolve, so are
    /// not reported as unresolved.
    pub ignore_unresolved: Vec<NamePattern>,
}

impl HaskellConfig {
    /// Parse the table of Haskell. An invalid table has the defaults, so that
    /// the language is still enabled.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<HaskellTable>() else {
            return Self::default();
        };
        Self {
            include: parsed.include.map(|include| table.patterns(&include, Glob::new)),
            ignore_unresolved: table.patterns(&parsed.ignore_unresolved, NamePattern::new),
        }
    }
}

impl LanguageOptions for HaskellConfig {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }

    fn backend(&self, settings: BackendSettings<'_>) -> Box<dyn Backend> {
        Box::new(HaskellBackend::new(
            settings.root,
            settings.file_system.clone(),
            self.ignore_unresolved.clone(),
        ))
    }
}

/// Haskell.
// The table as written, which is its schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
struct HaskellTable {
    /// Globs of the files to scan. Defaults to every Haskell source, plain
    /// and literate, with the boot files, the signatures and the sources of
    /// `hsc2hs`: `**/*.hs`, `**/*.lhs`, `**/*.hs-boot`, `**/*.lhs-boot`,
    /// `**/*.hsig`, `**/*.lhsig` and `**/*.hsc`.
    include: Option<Vec<Located<String>>>,

    /// Name patterns of the specifiers of imports that are expected not to
    /// resolve, so are not reported as unresolved, such as `Generated.*`.
    ignore_unresolved: Vec<Located<String>>,
}
