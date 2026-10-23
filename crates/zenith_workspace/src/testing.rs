//! Helpers for tests that build git repositories: this crate's, and, with
//! the `testing` feature, other crates'.

use std::{path::Path, process::Command};

/// Run git in `dir`, outside any hook that could have set its variables.
pub fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// Commit every file in the repository at `root`, as a test author, without
/// signing.
pub fn commit(root: &Path) {
    git(root, &["add", "--all"]);
    let config = ["user.name=zenith", "user.email=zenith@example.com", "commit.gpgSign=false"];
    let mut commit: Vec<_> = config.iter().flat_map(|setting| ["-c", setting]).collect();
    commit.extend(["commit", "--quiet", "--message", "test"]);
    git(root, &commit);
}
