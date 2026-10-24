//! Helpers for tests that build git repositories: this crate's, and, with
//! the `testing` feature, other crates'.

use std::{
    path::Path,
    process::{Command, Output},
};

/// Run git in `dir`, outside any hook that could have set its variables.
pub fn git(dir: &Path, args: &[&str]) {
    run(dir, args);
}

/// What git prints when run in `dir` with `args`, as [`git`] runs it.
pub fn git_output(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(run(dir, args).stdout).unwrap()
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

/// Run git in `dir` with `args`, outside any hook that could have set its
/// variables, checking that it succeeds.
fn run(dir: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "git {args:?} failed: {error}");
    output
}
