//! The zenith pipeline: the stages that turn a project into its import graph,
//! the session they share, and the [`LANGUAGES`] they read.
//!
//! A [`Session`] holds one project: its root and configuration, and what each
//! stage has found in it so far. A [`Pipeline`] runs its registered
//! [`Stage`]s on a session in the order of their [`StageKind`]s, times each
//! one, and stops at the first error, or after the stage that a command needs:
//!
//! | Stage | Finds |
//! |---|---|
//! | [`Discover`] | the source files and the workspace packages |
//! | [`Parse`] | the imports, directives and suppressions of each file |
//! | [`Resolve`] | what each import's specifier resolves to |
//! | [`BuildGraph`] | the module tree and import graph |
//!
//! Each stage is a thin adapter over the crate that does its work, so those
//! crates stay independent libraries, and only this crate knows the order they
//! run in. Files that can't be read or parsed, and imports that don't resolve,
//! don't stop the pipeline: they are left out of the graph, and recorded in
//! the session as [`FileError`]s and [`UnresolvedImport`]s.
//!
//! The [`Parse`] stage can keep what it finds in a cache, so that the next
//! run neither reads nor parses the files that haven't changed. Set
//! [`Session::cache`] to use one.

mod backends;
mod error;
mod languages;
mod pipeline;
mod schema;
mod scope;
mod session;
mod stages;
#[cfg(test)]
mod tests;

pub use backends::Backends;
pub use error::{FileError, FileErrorKind, PipelineError, UnresolvedImport};
pub use languages::LANGUAGES;
pub use pipeline::{Pipeline, Stage, StageKind};
pub use schema::{FileProblemJson, UnresolvedJson};
pub use scope::Scope;
pub use session::{FileSuppressions, Graphs, ResolvedImport, Session, SourceFile, Stats};
pub use stages::{BuildGraph, Discover, Parse, Resolve};
