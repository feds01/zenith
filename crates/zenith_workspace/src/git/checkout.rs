//! Checking out a commit of a project's repository into a temporary
//! directory, for reading the project as it was at that commit.

use std::{
    fs, io, mem,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
};

use log::warn;
use zenith_diagnostics::message;
use zenith_utils::path::{adjust_canonicalisation, relative_posix};

use super::{Git, GitError};

/// The settings that a checkout runs git with, so that it writes nothing into
/// the repository and reaches no server: no shared index, no file system
/// monitor, and Git LFS files left as their pointers.
const ISOLATED: [&str; 10] = [
    "-c",
    "core.splitIndex=false",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "filter.lfs.smudge=",
    "-c",
    "filter.lfs.process=",
    "-c",
    "filter.lfs.required=false",
];

/// The temporary directories of the checkouts that haven't been dropped yet.
static HELD: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

impl Git {
    /// Check out the commit that `reference` names, e.g. `main` or `HEAD~2`,
    /// into a new temporary directory, which is removed when the
    /// [`Checkout`] is dropped, or by [`remove_checkouts`]. The reference is
    /// resolved as by [`Self::resolve_commit`], with its errors.
    ///
    /// The files are written from an index of the checkout's own, so neither
    /// the repository's index nor its list of worktrees changes: nothing is
    /// left behind in the repository, even if zenith is killed before it
    /// removes the directory. Nor does git fetch anything: the files that Git
    /// LFS stores are checked out as their pointers.
    pub fn checkout(&self, reference: &str) -> Result<Checkout, GitError> {
        // 1. Find the commit, so that a reference that names none says so.
        let commit = self.resolve_commit(reference)?;

        // 2. Make the temporary directory, which the checkout removes from here on.
        let temporary = |error: io::Error| GitError::Checkout(error.to_string());
        let dir = tempfile::Builder::new().prefix("zenith-").tempdir().map_err(temporary)?;
        let dir = adjust_canonicalisation(dir.keep());
        held().push(dir.clone());
        let checkout =
            Checkout { commit, worktree: self.worktree.clone(), root: dir.join("tree"), dir };
        fs::create_dir(&checkout.root).map_err(temporary)?;

        // 3. Read the commit's tree into the checkout's index, and write its files out
        //    of it into the checkout's root.
        let mut git = self.clone();
        git.variables.push(("GIT_INDEX_FILE", checkout.dir.join("index").into_os_string()));
        git.variables.push(("GIT_WORK_TREE", checkout.root.clone().into_os_string()));
        let mut read_tree = ISOLATED.to_vec();
        read_tree.extend(["read-tree", &checkout.commit]);
        git.run(&read_tree)?;
        let mut checkout_index = ISOLATED.to_vec();
        checkout_index.extend(["checkout-index", "--all", "--force"]);
        git.run(&checkout_index)?;
        Ok(checkout)
    }
}

/// Remove the directory of every checkout that hasn't been dropped, as a
/// crash handler that exits without unwinding the stack, and so without
/// dropping them, must.
pub fn remove_checkouts() {
    for dir in mem::take(&mut *held()) {
        let _ = fs::remove_dir_all(dir);
    }
}

/// The directories of the checkouts that haven't been dropped yet.
fn held() -> MutexGuard<'static, Vec<PathBuf>> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A commit of a project's repository, checked out into a temporary directory
/// by [`Git::checkout`]. The directory is removed when the checkout is
/// dropped, on every path out of the code that holds it, an error's or a
/// panic's included.
#[derive(Debug)]
pub struct Checkout {
    /// The id of the commit.
    commit: String,

    /// The root of the worktree that the commit was checked out from.
    worktree: PathBuf,

    /// The root of the checkout, which is canonical. Each file of the commit
    /// is at the same path relative to it as relative to the worktree's root.
    root: PathBuf,

    /// The temporary directory, which holds the checkout's root and its
    /// index, and is removed with them when the checkout is dropped.
    dir: PathBuf,
}

impl Checkout {
    /// The id of the commit.
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// The path in the checkout of `path`, which is canonical: the same path
    /// relative to the checkout's root as `path` is relative to the
    /// worktree's, or `path` itself outside the worktree, which the commit
    /// doesn't hold.
    pub fn path_of(&self, path: &Path) -> PathBuf {
        match relative_posix(&self.worktree, path) {
            Some(name) if name.is_empty() => self.root.clone(),
            Some(name) => self.root.join(name),
            None => path.to_owned(),
        }
    }
}

impl Drop for Checkout {
    fn drop(&mut self) {
        held().retain(|dir| *dir != self.dir);
        if let Err(error) = fs::remove_dir_all(&self.dir) {
            let path = self.dir.display();
            warn!("{}", message!(workspace_checkout_not_removed, path, error));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        git::tests::committed,
        testing::{commit, git, git_output},
    };

    /// The worktrees that git lists for the repository at `root`.
    fn worktrees(root: &Path) -> String {
        git_output(root, &["worktree", "list", "--porcelain"])
    }

    #[test]
    fn checks_out_a_commit_and_removes_it_when_dropped() {
        let (_dir, root) = committed(&["web/src/a.ts", "web/b.ts"]);
        fs::remove_file(root.join("web/b.ts")).unwrap();
        fs::write(root.join("web/c.ts"), "").unwrap();
        let before = worktrees(&root);

        let checkout = Git::new(&root).checkout("HEAD").unwrap();
        let web = checkout.path_of(&root.join("web"));
        assert!(web.join("src/a.ts").is_file());
        assert!(web.join("b.ts").is_file());
        assert!(!web.join("c.ts").exists());
        assert_eq!(checkout.path_of(&root), checkout.root);
        assert_eq!(checkout.commit().len(), 40);

        // Neither the repository's worktrees nor its index change.
        assert_eq!(worktrees(&root), before);
        git(&root, &["diff", "--cached", "--quiet"]);

        // The crash handler would remove it until it is dropped.
        let temporary = checkout.dir.clone();
        assert!(held().contains(&temporary));
        drop(checkout);
        assert!(!temporary.exists());
        assert!(!held().contains(&temporary));
    }

    #[test]
    fn checks_out_a_commit_from_a_root_inside_the_repository() {
        let (_dir, repository) = committed(&["web/a.ts", "api/b.ts"]);
        let checkout = Git::new(&repository.join("web")).checkout("HEAD").unwrap();
        let web = checkout.path_of(&repository.join("web"));
        assert!(web.join("a.ts").is_file());
        // A file outside the repository is the same at every commit.
        let elsewhere = repository.parent().unwrap().join("zenith.toml");
        assert_eq!(checkout.path_of(&elsewhere), elsewhere);
    }

    #[test]
    fn checks_out_without_running_git_lfs() {
        let (_dir, root) = committed(&["a.ts"]);
        fs::write(root.join(".gitattributes"), "*.ts filter=lfs\n").unwrap();
        commit(&root);
        // A filter that fails, as Git LFS does when it can't reach its server.
        for (name, value) in [
            ("filter.lfs.smudge", "false"),
            ("filter.lfs.process", "false"),
            ("filter.lfs.required", "true"),
        ] {
            git(&root, &["config", name, value]);
        }

        let checkout = Git::new(&root).checkout("HEAD").unwrap();
        assert!(checkout.path_of(&root).join("a.ts").is_file());
    }

    #[test]
    fn a_reference_that_names_no_commit_is_unknown() {
        let (_dir, root) = committed(&["a.ts"]);
        // `-p` is not taken for an option of git's.
        for reference in ["no-such-branch", "-p", "HEAD~5"] {
            let error = Git::new(&root).checkout(reference).unwrap_err();
            assert_eq!(error, GitError::UnknownCommit(reference.to_owned()), "{reference}");
        }

        let dir = tempfile::tempdir().unwrap();
        let root = adjust_canonicalisation(dir.path());
        // A repository above the temporary directory would know its commits.
        if !root.ancestors().any(|dir| dir.join(".git").exists()) {
            let error = Git::new(&root).checkout("HEAD").unwrap_err();
            assert_eq!(error, GitError::NotARepository(root));
        }
    }
}
