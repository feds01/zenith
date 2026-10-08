//! The files and workspace packages of a zenith project.
//!
//! [walk] finds every file to scan, in parallel, respecting `.gitignore` and
//! zenith's default excludes. [discover_packages] reads the workspace
//! package declarations (`package.json` `workspaces`, or
//! `pnpm-workspace.yaml`) and matches them against the `package.json` files
//! the walk found.

mod packages;
mod walk;

pub use packages::{Workspace, WorkspacePackage, discover_packages};
pub use walk::{WalkResult, WalkSettings, walk};

/// Directories that are never scanned, wherever they appear.
pub const ALWAYS_EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".next",
    ".nuxt",
    ".output",
    ".svelte-kit",
    ".turbo",
    ".vercel",
    ".cache",
    ".wxt",
    ".mastra",
    "coverage",
    "storybook-static",
];

/// Build output directories, which are skipped when they sit directly inside
/// a package (next to a `package.json`). Elsewhere, e.g. `src/build/`, they
/// may well be source code.
pub const PACKAGE_OUTPUT_DIRS: &[&str] = &["dist", "build", "out"];
