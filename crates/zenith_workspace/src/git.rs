//! The files that git tracks, for scanning only those, as in a pre-commit
//! hook, where untracked files aren't part of the commit.

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
    process::Command,
};

use rustc_hash::FxHashSet;
use zenith_utils::path::relative_posix;

use crate::WalkResult;

/// The files that git tracks in a worktree: those in its index, which is the
/// next commit's while a pre-commit hook runs.
#[derive(Debug, Clone)]
pub struct TrackedFiles {
    /// The worktree's root, the nearest directory with a `.git`.
    worktree: PathBuf,

    /// The POSIX paths of the files, relative to `worktree`.
    names: FxHashSet<String>,
}

impl TrackedFiles {
    /// The files that git tracks in the worktree that `root`, which must be
    /// canonical, is in.
    pub fn find(root: &Path) -> Result<Self, GitError> {
        Self::find_with(root, [])
    }

    /// [`Self::find`], running git with the extra environment `variables`.
    fn find_with<'a>(
        root: &Path,
        variables: impl IntoIterator<Item = (&'a str, &'a OsStr)>,
    ) -> Result<Self, GitError> {
        // Git runs hooks in the worktree's root, and sets some of its variables to
        // paths relative to it, such as `GIT_INDEX_FILE=.git/index`, so run git
        // there too. Outside a worktree, git explains the problem.
        let worktree = root.ancestors().find(|dir| dir.join(".git").exists()).unwrap_or(root);
        let output = Command::new("git")
            .current_dir(worktree)
            .args(["ls-files", "-z", "--cached"])
            .envs(variables)
            .output()
            .map_err(|error| GitError::Run(error.to_string()))?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(GitError::Failed(message));
        }
        // `ls-files` lists the files relative to the directory it runs in, and `-z`
        // keeps their names as they are, without quoting.
        let names = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect();
        Ok(Self { worktree: worktree.to_owned(), names })
    }

    /// Whether git tracks the file at `path`, which is absolute.
    pub fn contains(&self, path: &Path) -> bool {
        relative_posix(&self.worktree, path).is_some_and(|name| self.names.contains(&name))
    }
}

impl WalkResult {
    /// Keep only the files that git tracks, so that untracked source files
    /// aren't scanned, and untracked packages aren't found.
    pub fn retain_tracked(&mut self, tracked: &TrackedFiles) {
        self.source_files.retain(|path| tracked.contains(path));
        self.package_jsons.retain(|path| tracked.contains(path));
    }
}

/// Why the files that git tracks couldn't be listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// `git` could not be run, most likely because it isn't installed, with
    /// this message.
    Run(String),

    /// `git ls-files` failed, most likely outside a repository, with this
    /// message.
    Failed(String),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(error) => write!(f, "could not run `git` to list the tracked files: {error}"),
            Self::Failed(message) => {
                write!(f, "could not list the files that git tracks: {message}")
            }
        }
    }
}

impl std::error::Error for GitError {}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::path::adjust_canonicalisation;

    use super::*;

    /// Run git in `dir`, outside any hook that could have set its variables.
    fn git(dir: &Path, args: &[&str]) {
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

    /// A git repository with `files`, of which only `tracked` are added to
    /// the index, and its canonical root.
    fn repository(files: &[&str], tracked: &[&str]) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        git(&root, &["init", "--quiet"]);
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        let mut add = vec!["add", "--"];
        add.extend(tracked);
        git(&root, &add);
        (dir, root)
    }

    #[test]
    fn untracked_files_are_left_out() {
        let (_dir, root) = repository(
            &["src/a.ts", "src/b.ts", "scratch.ts", "package.json", "new/package.json"],
            &["src/a.ts", "package.json"],
        );
        let tracked = TrackedFiles::find(&root).unwrap();
        assert!(tracked.contains(&root.join("src/a.ts")));
        assert!(!tracked.contains(&root.join("src/b.ts")));

        let mut result = WalkResult {
            source_files: ["src/a.ts", "src/b.ts", "scratch.ts"].map(|file| root.join(file)).into(),
            package_jsons: ["package.json", "new/package.json"].map(|file| root.join(file)).into(),
        };
        result.retain_tracked(&tracked);
        assert_eq!(result.source_files, [root.join("src/a.ts")]);
        assert_eq!(result.package_jsons, [root.join("package.json")]);
    }

    #[test]
    fn files_are_named_relative_to_a_root_inside_the_repository() {
        let (_dir, repository) = repository(&["web/src/a.ts", "web/b.ts"], &["web/src/a.ts"]);
        let root = repository.join("web");
        let tracked = TrackedFiles::find(&root).unwrap();
        assert!(tracked.contains(&root.join("src/a.ts")));
        assert!(!tracked.contains(&root.join("b.ts")));
    }

    #[test]
    fn listing_fails_outside_a_repository() {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        // A repository above the temporary directory would make it pass.
        if root.ancestors().any(|dir| dir.join(".git").exists()) {
            return;
        }
        let error = TrackedFiles::find(&root).unwrap_err();
        assert!(matches!(error, GitError::Failed(_)), "{error:?}");
    }

    #[test]
    fn a_relative_index_from_a_hook_is_found_from_a_subdirectory() {
        // Git sets `GIT_INDEX_FILE=.git/index` for the hooks of a plain commit,
        // relative to the worktree's root, where it runs them.
        let (_dir, repository) = repository(&["web/src/a.ts", "web/b.ts"], &["web/src/a.ts"]);
        let root = repository.join("web");
        let index = [("GIT_INDEX_FILE", OsStr::new(".git/index"))];
        let tracked = TrackedFiles::find_with(&root, index).unwrap();
        assert!(tracked.contains(&root.join("src/a.ts")));
        assert!(!tracked.contains(&root.join("b.ts")));
    }
}
