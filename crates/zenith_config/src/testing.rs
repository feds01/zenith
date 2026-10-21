//! A language for the tests of this crate, which knows none of the real
//! ones: they are defined in their own crates.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_utils::glob::{Glob, NamePattern};

use crate::{
    language::{Language, LanguageOptions, Languages},
    parse::LanguageTable,
    schema::Located,
};

/// The language of the tests. Its files are `.tst` files, and its schemes are
/// `ext:` for external packages, and `core:` and `sys:` for builtins.
pub(crate) const TEST: Language = Language {
    name: "test",
    table_names: &["test", "testing"],
    description: "The language of the tests.",
    enabled_by_default: true,
    include: &["**/*.tst"],
    source_extensions: &["tst"],
    optional_extensions: &["tst", "d.tst"],
    schemes: &["ext", "core", "sys"],
    parse: |table| Arc::new(TestOptions::parse(table)),
    defaults: || Arc::new(TestOptions::default()),
    schema: |generator| generator.subschema_for::<TestTable>(),
};

/// A language that is only enabled by its table.
pub(crate) const OTHER: Language = Language {
    name: "other",
    table_names: &["other"],
    description: "Another language of the tests.",
    enabled_by_default: false,
    include: &["**/*.oth"],
    source_extensions: &["oth"],
    optional_extensions: &["oth"],
    schemes: &["oth"],
    parse: |table| Arc::new(OtherOptions::parse(table)),
    defaults: || Arc::new(OtherOptions::default()),
    schema: |generator| generator.subschema_for::<OtherTable>(),
};

/// The languages of the tests.
pub(crate) const LANGUAGES: Languages = Languages::new(&[&TEST, &OTHER]);

/// The options of the test language.
#[derive(Debug, Default)]
pub(crate) struct TestOptions {
    /// Globs of the files to scan.
    pub(crate) include: Option<Vec<Glob>>,

    /// Name patterns, which are checked like any other.
    pub(crate) names: Vec<NamePattern>,
}

impl TestOptions {
    /// Parse the table of the test language.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<TestTable>() else {
            return Self::default();
        };
        Self {
            include: parsed.include.map(|include| table.patterns(&include, Glob::new)),
            names: table.patterns(&parsed.names, NamePattern::new),
        }
    }
}

impl LanguageOptions for TestOptions {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }
}

/// The test language.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
pub(crate) struct TestTable {
    /// Globs of the files to scan.
    pub(crate) include: Option<Vec<Located<String>>>,

    /// Name patterns.
    pub(crate) names: Vec<Located<String>>,
}

/// The options of the other language.
#[derive(Debug, Default)]
pub(crate) struct OtherOptions {
    /// Globs of the files to scan.
    pub(crate) include: Option<Vec<Glob>>,
}

impl OtherOptions {
    /// Parse the table of the other language.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<OtherTable>() else {
            return Self::default();
        };
        Self { include: parsed.include.map(|include| table.patterns(&include, Glob::new)) }
    }
}

impl LanguageOptions for OtherOptions {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }
}

/// The other language.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
pub(crate) struct OtherTable {
    /// Globs of the files to scan.
    pub(crate) include: Option<Vec<Located<String>>>,
}
