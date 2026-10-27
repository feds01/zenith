//! A language for the tests of this crate, which knows none of the real
//! ones: they are defined in their own crates, and a walk of a project, which
//! is the pipeline's.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_lang::{Backend, BackendSettings};
use zenith_utils::{
    fs::{FileKind, FileSystem},
    glob::{Glob, NamePattern},
    path::relative_posix,
};

use crate::{
    config::{FilesConfig, ProjectWalk},
    language::{Language, LanguageOptions, Languages},
    parse::LanguageTable,
    schema::Located,
};

/// The language of the tests. Its files are `.tst` files, and its schemes are
/// `ext:` for external packages, and `core:` and `sys:` for builtins, whose
/// `core:` names are paths written with `::`.
pub(crate) const TEST: Language = Language {
    name: "test",
    table_names: &["test", "testing"],
    description: "The language of the tests.",
    enabled_by_default: true,
    include: &["**/*.tst"],
    source_extensions: &["tst"],
    source_file_names: &[],
    optional_extensions: &["tst", "d.tst"],
    directory_modules: &[],
    index_files: &["mod.tst"],
    schemes: &["ext", "core", "sys"],
    path_schemes: &[("core", "::")],
    parse: |table| Arc::new(TestOptions::parse(table)),
    defaults: || Arc::new(TestOptions::default()),
    schema: |generator| generator.subschema_for::<TestTable>(),
};

/// A language that is only enabled by its table. Its files are `.oth` files,
/// and the files named `Otherfile`.
pub(crate) const OTHER: Language = Language {
    name: "other",
    table_names: &["other"],
    description: "Another language of the tests.",
    enabled_by_default: false,
    include: &["**/*.oth", "**/Otherfile"],
    source_extensions: &["oth"],
    source_file_names: &["Otherfile"],
    optional_extensions: &["oth"],
    directory_modules: &[],
    index_files: &[],
    schemes: &["oth"],
    path_schemes: &[],
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

    fn backend(&self, _: BackendSettings<'_>) -> Box<dyn Backend> {
        unreachable!("the tests of the configuration don't run the pipeline")
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

    fn backend(&self, _: BackendSettings<'_>) -> Box<dyn Backend> {
        unreachable!("the tests of the configuration don't run the pipeline")
    }
}

/// The other language.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
pub(crate) struct OtherTable {
    /// Globs of the files to scan.
    pub(crate) include: Option<Vec<Located<String>>>,
}

/// A walk of every file of a project that the `exclude` globs don't skip,
/// which knows nothing of `.gitignore`.
pub(crate) struct TestWalk;

impl ProjectWalk for TestWalk {
    fn files(
        &self,
        file_system: &dyn FileSystem,
        root: &Path,
        files: &FilesConfig,
    ) -> Vec<PathBuf> {
        walk_files(file_system, root, &files.exclude)
    }
}

/// The files of the project at `root`, in `file_system`, that the `exclude`
/// globs don't skip, as [`TestWalk`] finds them.
pub(crate) fn walk_files(
    file_system: &dyn FileSystem,
    root: &Path,
    exclude: &[Glob],
) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in file_system.read_dir(&dir).unwrap_or_default() {
            let Some(relative) = relative_posix(root, &entry.path) else {
                continue;
            };
            if exclude.iter().any(|glob| glob.is_match(&relative)) {
                continue;
            }
            match entry.kind {
                FileKind::Directory => pending.push(entry.path),
                _ => found.push(entry.path),
            }
        }
    }
    found
}
