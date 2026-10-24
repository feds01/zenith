//! The files that git tracks, for scanning only those, as in a pre-commit
//! hook, where untracked files aren't part of the commit.

use std::path::{Path, PathBuf};

use rustc_hash::FxHashSet;
use zenith_utils::path::relative_posix;

use super::{Git, GitError, fields};
use crate::WalkResult;

impl Git {
    /// The files that git tracks in the worktree: those in its index, which
    /// is the next commit's while a pre-commit hook runs.
    pub fn tracked_files(&self) -> Result<TrackedFiles, GitError> {
        self.ensure_repository()?;
        let output = self.run(&["ls-files", "-z", "--cached"])?;
        // `ls-files` lists the files relative to the directory it runs in.
        let names = fields(&output).map(str::to_owned).collect();
        Ok(TrackedFiles { worktree: self.worktree.clone(), names })
    }
}

/// The files that git tracks in a worktree, as [`Git::tracked_files`] finds
/// them.
#[derive(Debug, Clone)]
pub struct TrackedFiles {
    /// The worktree's root.
    worktree: PathBuf,

    /// The POSIX paths of the files, relative to `worktree`.
    names: FxHashSet<String>,
}

impl TrackedFiles {
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
        self.manifests.retain(|path| tracked.contains(path));
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use zenith_utils::path::adjust_canonicalisation;

    use super::*;
    use crate::git::tests::repository;

    #[test]
    fn untracked_files_are_left_out() {
        let (_dir, root) = repository(
            &["src/a.ts", "src/b.ts", "scratch.ts", "package.json", "new/package.json"],
            &["src/a.ts", "package.json"],
        );
        let tracked = Git::new(&root).tracked_files().unwrap();
        assert!(tracked.contains(&root.join("src/a.ts")));
        assert!(!tracked.contains(&root.join("src/b.ts")));

        let mut result = WalkResult {
            source_files: ["src/a.ts", "src/b.ts", "scratch.ts"].map(|file| root.join(file)).into(),
            manifests: ["package.json", "new/package.json"].map(|file| root.join(file)).into(),
        };
        result.retain_tracked(&tracked);
        assert_eq!(result.source_files, [root.join("src/a.ts")]);
        assert_eq!(result.manifests, [root.join("package.json")]);
    }

    #[test]
    fn files_are_named_relative_to_a_root_inside_the_repository() {
        let (_dir, repository) = repository(&["web/src/a.ts", "web/b.ts"], &["web/src/a.ts"]);
        let root = repository.join("web");
        let tracked = Git::new(&root).tracked_files().unwrap();
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
        let error = Git::new(&root).tracked_files().unwrap_err();
        assert_eq!(error, GitError::NotARepository(root));
    }

    #[test]
    fn a_relative_index_from_a_hook_is_found_from_a_subdirectory() {
        // Git sets `GIT_INDEX_FILE=.git/index` for the hooks of a plain commit,
        // relative to the worktree's root, where it runs them.
        let (_dir, repository) = repository(&["web/src/a.ts", "web/b.ts"], &["web/src/a.ts"]);
        let root = repository.join("web");
        let git = Git::new(&root).with_env("GIT_INDEX_FILE", ".git/index");
        let tracked = git.tracked_files().unwrap();
        assert!(tracked.contains(&root.join("src/a.ts")));
        assert!(!tracked.contains(&root.join("b.ts")));
    }
}
