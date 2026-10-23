//! Benchmarks for the zenith crates, and the inputs that they run on.
//!
//! The benchmarks are in `benches/`, and the README says how to run them.
//! Their input is a [`SyntheticRepo`]: a TypeScript monorepo generated
//! deterministically at any size, so that scaling is measured without
//! committing a large fixture. Benchmarks that read from disk write one to a
//! temporary directory with [`TempRepo`].

mod synthetic_repo;
mod temp_repo;

pub use synthetic_repo::{
    FileKind, ImportTarget, MIN_SOURCE_FILES, PackageRole, SyntheticFile, SyntheticImport,
    SyntheticPackage, SyntheticRepo,
};
pub use temp_repo::{RemoveShared, TempRepo};
use zenith_config::{FilesConfig, Languages, LanguagesConfig};
pub use zenith_graph::ImportKind;
use zenith_lang_js::JS;
use zenith_workspace::WalkSettings;

/// The walk settings of a JavaScript project without a `[files]` or
/// `[languages]` table.
pub fn default_walk_settings() -> WalkSettings {
    let FilesConfig { exclude, respect_gitignore } = FilesConfig::default();
    let scanned = LanguagesConfig::defaults(Languages::new(&[&JS])).scanned();
    WalkSettings {
        include: scanned.globs,
        exclude,
        respect_gitignore,
        source_extensions: scanned.extensions,
        source_file_names: scanned.file_names,
        manifests: vec!["package.json"],
    }
}
