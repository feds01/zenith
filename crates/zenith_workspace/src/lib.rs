//! The files and workspace packages of a zenith project.
//!
//! [`walk`] finds every file to scan, in parallel, respecting `.gitignore` and
//! zenith's default excludes. [`discover_packages`] reads the workspace
//! package declarations (`package.json` `workspaces`, or
//! `pnpm-workspace.yaml`) and matches them against the `package.json` files
//! that the walk found. A [`Git`] client finds the files that git tracks,
//! which narrow the walk down, and the files that it sees renamed.

mod git;
mod packages;
mod walk;

pub use git::{Git, GitError, Renames, TrackedFiles};
pub use packages::{Workspace, WorkspacePackage, discover_packages};
pub use walk::{WalkResult, WalkSettings, walk};
