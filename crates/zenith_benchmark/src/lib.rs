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
use zenith_config::{FilesConfig, SOURCE_EXTENSIONS};
pub use zenith_graph::ImportKind;
use zenith_workspace::WalkSettings;

/// The walk settings of a project without a `[files]` section.
pub fn default_walk_settings() -> WalkSettings {
    let FilesConfig { include, exclude, respect_gitignore } = FilesConfig::default();
    WalkSettings { include, exclude, respect_gitignore, source_extensions: SOURCE_EXTENSIONS }
}
