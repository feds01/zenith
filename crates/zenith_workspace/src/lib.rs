//! The files of a zenith project.
//!
//! [`walk`] finds every file to scan, in parallel, respecting `.gitignore` and
//! zenith's default excludes. A [`Git`] client finds the files that git tracks,
//! which narrow the walk down, and the files that it sees renamed, and
//! reads the files of a commit as a [`CommitFileSystem`], which the walk can
//! walk too.

mod git;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod walk;

pub use git::{CommitFileSystem, Git, GitError, Renames, TrackedFiles};
pub use walk::{
    WalkDependencies, WalkResult, WalkSettings, walk, walk_dependencies_outside, walk_recording,
};
