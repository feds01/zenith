//! Benchmarks for the zenith crates, and the inputs that they run on.
//!
//! Their input is a [`SyntheticRepo`]: a TypeScript monorepo generated
//! deterministically at any size, so that scaling is measured without
//! committing a large fixture.

mod synthetic_repo;

pub use synthetic_repo::{
    FileKind, ImportTarget, MIN_SOURCE_FILES, PackageRole, SyntheticFile, SyntheticImport,
    SyntheticPackage, SyntheticRepo,
};
pub use zenith_graph::ImportKind;
