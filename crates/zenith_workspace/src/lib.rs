//! The files of a zenith project.
//!
//! [`walk`] finds every file to scan, in parallel, respecting `.gitignore` and
//! zenith's default excludes. A [`Git`] client finds the files that git tracks,
//! which narrow the walk down, and the files that it sees renamed, and
//! checks a commit out into a temporary [`Checkout`].

mod git;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod walk;

pub use git::{Checkout, CommitFileSystem, Git, GitError, Renames, TrackedFiles};
pub use walk::{WalkResult, WalkSettings, walk};
