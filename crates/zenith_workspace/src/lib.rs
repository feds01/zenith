//! The files and workspace packages of a zenith project.
//!
//! [`walk`] finds every file to scan, in parallel, respecting `.gitignore` and
//! zenith's default excludes. [`discover_packages`] reads the workspace
//! package declarations (`package.json` `workspaces`, or
//! `pnpm-workspace.yaml`) and matches them against the `package.json` files
//! that the walk found. [`TrackedFiles`] narrows the walk down to the files
//! that git tracks.

mod git;
mod packages;
mod walk;

pub use git::{GitError, TrackedFiles};
pub use packages::{Workspace, WorkspacePackage, discover_packages};
pub use walk::{WalkResult, WalkSettings, walk};
