//! Go in the configuration: the [`GO`] language, the `[languages.go]` table,
//! and the options it is parsed into.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_config::{Language, LanguageOptions, LanguageTable, Located};
use zenith_lang::{Backend, BackendSettings};
use zenith_utils::glob::Glob;

use crate::backend::GoBackend;

/// Go.
pub const GO: Language = Language {
    name: "go",
    table_names: &["go"],
    description: "Go: how its files are found and their imports resolved.",
    enabled_by_default: false,
    include: &["**/*.go"],
    source_extensions: &["go"],
    optional_extensions: &["go"],
    // `go:` names the standard library, and `gomod:` the modules of the
    // dependencies.
    schemes: &["go", "gomod"],
    parse: |table| Arc::new(GoConfig::parse(table)),
    defaults: || Arc::new(GoConfig::default()),
    schema: |generator| generator.subschema_for::<GoTable>(),
};

/// The options of Go. `None` options leave the defaults in place.
#[derive(Debug, Clone, Default)]
pub struct GoConfig {
    /// Globs of the files to scan. `None` scans every file with one of the
    /// [`GO`] source extensions.
    pub include: Option<Vec<Glob>>,
}

impl GoConfig {
    /// Parse the table of Go. An invalid table has the defaults, so that the
    /// language is still enabled.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<GoTable>() else {
            return Self::default();
        };
        Self { include: parsed.include.map(|include| table.patterns(&include, Glob::new)) }
    }
}

impl LanguageOptions for GoConfig {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }

    fn backend(&self, settings: BackendSettings<'_>) -> Box<dyn Backend> {
        Box::new(GoBackend::new(settings.root))
    }
}

/// Go.
// The table as written, which is its schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
struct GoTable {
    /// Globs of the files to scan. Defaults to every Go file, `**/*.go`.
    include: Option<Vec<Located<String>>>,
}
