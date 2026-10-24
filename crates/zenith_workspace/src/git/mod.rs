//! What git knows about a project, through a [`Git`] client: the files that
//! it tracks, for scanning only those, as in a pre-commit hook, where
//! untracked files aren't part of the commit, the files that it sees
//! renamed, for following them, the files that changed since a commit, for
//! reporting on only those, and the project at another commit, through a
//! [`Checkout`].

mod checkout;
mod commits;
mod renames;
mod tracked;

use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    process::Command,
};

pub use checkout::Checkout;
pub use renames::Renames;
pub use tracked::TrackedFiles;
use zenith_diagnostics::message;
use zenith_utils::path::relative_posix;

/// Runs git for a project, in the root of the worktree that the project is
/// in.
///
/// Git runs hooks in the worktree's root, and sets some of their variables to
/// paths relative to it, such as `GIT_INDEX_FILE=.git/index`, so the client
/// runs git there too, whichever directory of the worktree the project is
/// rooted at.
#[derive(Debug, Clone)]
pub struct Git {
    /// The project root, which is canonical.
    root: PathBuf,

    /// The worktree's root: the nearest directory with a `.git`, at or above
    /// the project root, or the project root outside a worktree.
    worktree: PathBuf,

    /// What the project's paths start with to be relative to the worktree's
    /// root, as [`Self::root_prefix`] gives it, or `None` outside a worktree,
    /// which [`Self::ensure_repository`] reports as
    /// [`GitError::NotARepository`].
    prefix: Option<String>,

    /// The environment variables to run git with, besides zenith's own.
    variables: Vec<(&'static str, OsString)>,
}

impl Git {
    /// A client for the project at `root`, which must be canonical.
    pub fn new(root: &Path) -> Self {
        let worktree = root.ancestors().find(|dir| dir.join(".git").exists());
        let prefix = worktree
            .and_then(|worktree| relative_posix(worktree, root))
            .map(|prefix| if prefix.is_empty() { prefix } else { format!("{prefix}/") });
        Self {
            root: root.to_owned(),
            worktree: worktree.unwrap_or(root).to_owned(),
            prefix,
            variables: Vec::new(),
        }
    }

    /// This client, running git with the environment variable `name` set to
    /// `value`, as a hook would.
    #[cfg(test)]
    fn with_env(mut self, name: &'static str, value: &str) -> Self {
        self.variables.push((name, value.into()));
        self
    }

    /// What the project's paths start with to be relative to the root of the
    /// worktree that it is in: the root's POSIX path in it, ending in `/`,
    /// or nothing if the project is rooted there. `None` outside a worktree.
    pub fn root_prefix(&self) -> Option<String> {
        self.prefix.clone()
    }

    /// Fail with [`GitError::NotARepository`] if there is no `.git` at or
    /// above the project root, rather than with whatever git says, which
    /// depends on its version and language. Git still explains any other
    /// problem, such as a repository that it doesn't trust.
    fn ensure_repository(&self) -> Result<(), GitError> {
        match self.prefix {
            Some(_) => Ok(()),
            None => Err(GitError::NotARepository(self.root.clone())),
        }
    }

    /// Name the file `name`, which git names relative to the worktree's root,
    /// relative to the project root instead, which may be a directory inside
    /// it, or `None` if the file is outside the project root, or there is no
    /// worktree.
    fn inside(&self, name: &str) -> Option<String> {
        name.strip_prefix(self.prefix.as_deref()?).map(str::to_owned)
    }

    /// Run git with `args` in the worktree's root, returning what it printed.
    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let output = Command::new("git")
            .current_dir(&self.worktree)
            .args(args)
            .envs(self.variables.iter().map(|(name, value)| (name, value)))
            .output()
            .map_err(|error| GitError::Run(error.to_string()))?;
        if !output.status.success() {
            return Err(GitError::Failed {
                command: args.join(" "),
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// The fields of git's `-z` output, which separates them with NUL bytes, and
/// keeps names as they are, without quoting.
fn fields(output: &str) -> impl Iterator<Item = &str> {
    output.split('\0').filter(|field| !field.is_empty())
}

/// Why git couldn't tell zenith what it asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// `git` could not be run, most likely because it isn't installed, with
    /// this message.
    Run(String),

    /// Git ran but failed.
    Failed {
        /// The command's arguments, e.g. `ls-files -z --cached`.
        command: String,

        /// What git said.
        message: String,
    },

    /// The project, at this root, is not in a git repository's worktree.
    NotARepository(PathBuf),

    /// No commit has this name.
    UnknownCommit(String),

    /// The temporary directory to check a commit out into could not be made,
    /// with this message.
    Checkout(String),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(error) => f.write_str(&message!(workspace_git_not_run, error)),
            Self::Failed { command, message } => {
                f.write_str(&message!(workspace_git_failed, command, message))
            }
            Self::NotARepository(root) => {
                f.write_str(&message!(workspace_not_a_repository, root = root.display()))
            }
            Self::UnknownCommit(reference) => {
                f.write_str(&message!(workspace_unknown_commit, reference))
            }
            Self::Checkout(error) => f.write_str(&message!(workspace_checkout_failed, error)),
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
    use crate::testing::{commit, git};

    /// A git repository with `files`, each with contents of its own, so that
    /// git sees them renamed when they move, committed, and its canonical
    /// root.
    pub(super) fn committed(files: &[&str]) -> (TempDir, PathBuf) {
        let (dir, root) = repository(files, files);
        for file in files {
            fs::write(root.join(file), format!("export const name = '{file}';\n")).unwrap();
        }
        commit(&root);
        (dir, root)
    }

    /// A git repository with `files`, of which only `tracked` are added to
    /// the index, and its canonical root.
    pub(super) fn repository(files: &[&str], tracked: &[&str]) -> (TempDir, PathBuf) {
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
    fn a_root_inside_the_repository_has_its_path_as_a_prefix() {
        let (_dir, repository) = repository(&["web/src/a.ts"], &["web/src/a.ts"]);
        assert_eq!(Git::new(&repository).root_prefix().as_deref(), Some(""));
        assert_eq!(Git::new(&repository.join("web")).root_prefix().as_deref(), Some("web/"));
        let root = repository.join("web/src");
        assert_eq!(Git::new(&root).root_prefix().as_deref(), Some("web/src/"));

        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        // A repository above the temporary directory would give it a prefix.
        if !root.ancestors().any(|dir| dir.join(".git").exists()) {
            assert_eq!(Git::new(&root).root_prefix(), None);
        }
    }

    #[test]
    fn a_worktree_or_submodule_with_a_git_file_is_where_the_prefix_starts() {
        // The `.git` of a linked worktree or a submodule is a file naming its git
        // directory, elsewhere.
        let dir = TempDir::new().unwrap();
        let worktree = adjust_canonicalisation(dir.path()).join("module");
        fs::create_dir_all(worktree.join("web")).unwrap();
        fs::write(worktree.join(".git"), "gitdir: ../.git/modules/module\n").unwrap();
        assert_eq!(Git::new(&worktree).root_prefix().as_deref(), Some(""));
        assert_eq!(Git::new(&worktree.join("web")).root_prefix().as_deref(), Some("web/"));
    }

    #[test]
    fn git_explains_a_repository_that_it_cannot_read() {
        // A `.git` file that doesn't point at a repository.
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        fs::write(root.join(".git"), "not a link to a repository\n").unwrap();
        let git = Git::new(&root);
        for error in
            [git.tracked_files().unwrap_err(), git.changed_files("HEAD", true).unwrap_err()]
        {
            let GitError::Failed { message, .. } = &error else { panic!("{error:?}") };
            assert!(!message.is_empty(), "{error:?}");
        }
    }
}
