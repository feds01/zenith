//! Building crate `zenith`:
//!
//! Compute the commit information per build and inline it into the binary.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    // The workspace root directory is not available without walking up the tree
    // https://github.com/rust-lang/cargo/issues/3946
    let workspace_root =
        Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("..").join("..");

    commit_info(&workspace_root);
}

/// Run `git` in the workspace root, returning its trimmed stdout on success.
fn git(workspace_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").current_dir(workspace_root).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

/// The path of a file in the git directory, e.g. `HEAD`. Asking git (rather
/// than assuming `.git/HEAD`) also works in worktrees, where `.git` is a file
/// and refs live in the common directory.
fn git_path(workspace_root: &Path, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(git(workspace_root, &["rev-parse", "--git-path", name])?);
    Some(if path.is_absolute() { path } else { workspace_root.join(path) })
}

fn commit_info(workspace_root: &Path) {
    // If not in a git repository, do not attempt to retrieve commit information.
    let Some(head) = git_path(workspace_root, "HEAD") else {
        return;
    };

    // Rebuild when the commit changes. HEAD holds either a commit, when it is
    // detached, or `ref: <ref>` when on a branch. A commit on a branch writes
    // the branch's loose ref file, which may not exist beforehand once refs are
    // packed (so it cannot be watched), and always appends to HEAD's reflog,
    // `logs/HEAD`, which can.
    let mut watched = vec![head.clone()];
    watched.extend(git_path(workspace_root, "logs/HEAD"));
    if let Ok(contents) = fs::read_to_string(&head)
        && let Some(reference) = contents.trim().strip_prefix("ref: ")
    {
        watched.extend(git_path(workspace_root, reference));
        watched.extend(git_path(workspace_root, "packed-refs"));
    }
    // Watching a file that does not exist would rebuild every time.
    for path in watched.iter().filter(|path| path.exists()) {
        println!("cargo::rerun-if-changed={}", path.display());
    }

    let Some(log) =
        git(workspace_root, &["log", "-1", "--date=short", "--abbrev=9", "--format=%h %cd"])
    else {
        return;
    };
    let mut parts = log.split_whitespace();
    if let (Some(short_hash), Some(date)) = (parts.next(), parts.next()) {
        println!("cargo::rustc-env=ZENITH_COMMIT_SHORT_HASH={short_hash}");
        println!("cargo::rustc-env=ZENITH_COMMIT_DATE={date}");
    }
}
