//! The files of a zenith project.
//!
//! [`walk`] finds every file to scan, in parallel, respecting `.gitignore` and
//! zenith's default excludes. A [`Git`] client finds the files that git tracks,
//! which narrow the walk down, and the files that it sees renamed.

mod git;
mod walk;

pub use git::{Git, GitError, Renames, TrackedFiles};
pub use walk::{WalkResult, WalkSettings, walk};
