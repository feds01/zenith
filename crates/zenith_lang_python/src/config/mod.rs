//! Python in the configuration: the [`PYTHON`] language, the
//! `[languages.python]` table, and the options it is parsed into.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_config::{Language, LanguageOptions, LanguageTable, Located};
use zenith_lang::{Backend, BackendSettings};
use zenith_utils::glob::{Glob, NamePattern};

use crate::backend::PythonBackend;

/// Python.
pub const PYTHON: Language = Language {
    name: "python",
    table_names: &["python"],
    description: "Python: how its files are found and the imports between its modules resolved.",
    enabled_by_default: false,
    include: &["**/*.py", "**/*.pyi"],
    source_extensions: &["py", "pyi"],
    optional_extensions: &["py", "pyi"],
    // `pypi:` is for third-party packages, `python:` for the standard library.
    schemes: &["pypi", "python"],
    parse: |table| Arc::new(PythonConfig::parse(table)),
    defaults: || Arc::new(PythonConfig::default()),
    schema: |generator| generator.subschema_for::<PythonTable>(),
};

/// The options of Python. `None` options leave the defaults in place.
#[derive(Debug, Clone, Default)]
pub struct PythonConfig {
    /// Globs of the files to scan. `None` scans every file with one of the
    /// [`PYTHON`] source extensions.
    pub include: Option<Vec<Glob>>,

    /// The directories that absolute imports start from, relative to the
    /// project root. `None` finds them from the project's packages.
    pub roots: Option<Vec<String>>,

    /// Patterns of module paths that are expected not to resolve, so are not
    /// reported as unresolved.
    pub ignore_unresolved: Vec<NamePattern>,
}

impl PythonConfig {
    /// Parse the table of Python. An invalid table has the defaults, so that
    /// the language is still enabled.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<PythonTable>() else {
            return Self::default();
        };
        Self {
            include: parsed.include.map(|include| table.patterns(&include, Glob::new)),
            roots: parsed.roots,
            ignore_unresolved: table.patterns(&parsed.ignore_unresolved, NamePattern::new),
        }
    }
}

impl LanguageOptions for PythonConfig {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }

    fn backend(&self, settings: BackendSettings<'_>) -> Box<dyn Backend> {
        Box::new(PythonBackend::new(
            settings.root,
            self.roots.clone(),
            self.ignore_unresolved.clone(),
        ))
    }
}

/// Python.
// The table as written, which is its schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
struct PythonTable {
    /// Globs of the files to scan. Defaults to every Python file and stub,
    /// `**/*.py` and `**/*.pyi`.
    include: Option<Vec<Located<String>>>,

    /// The directories that absolute imports start from, relative to the
    /// project root, as on `sys.path`. Defaults to the `src` directory of each
    /// package with a `pyproject.toml`, or the package's own directory when it
    /// has none, and the project root.
    roots: Option<Vec<String>>,

    /// Name patterns of module paths that are expected not to resolve, so
    /// are not reported as unresolved.
    ignore_unresolved: Vec<Located<String>>,
}
