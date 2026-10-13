//! What git knows about a project, through a [`Git`] client: the files that
//! it tracks, for scanning only those, as in a pre-commit hook, where
//! untracked files aren't part of the commit, and the files that it sees
//! renamed, for following them.

use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    process::Command,
};

use log::debug;
use rustc_hash::{FxHashMap, FxHashSet};
use zenith_utils::path::relative_posix;

use crate::WalkResult;

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
    /// the project root, or the project root outside a worktree, where git
    /// explains the problem.
    worktree: PathBuf,

    /// The environment variables to run git with, besides zenith's own.
    variables: Vec<(&'static str, OsString)>,
}

impl Git {
    /// A client for the project at `root`, which must be canonical.
    pub fn new(root: &Path) -> Self {
        let worktree = root.ancestors().find(|dir| dir.join(".git").exists()).unwrap_or(root);
        Self { root: root.to_owned(), worktree: worktree.to_owned(), variables: Vec::new() }
    }

    /// This client, running git with the environment variable `name` set to
    /// `value`, as a hook would.
    #[cfg(test)]
    fn with_env(mut self, name: &'static str, value: &str) -> Self {
        self.variables.push((name, value.into()));
        self
    }

    /// The files that git tracks in the worktree: those in its index, which
    /// is the next commit's while a pre-commit hook runs.
    pub fn tracked_files(&self) -> Result<TrackedFiles, GitError> {
        let output = self.run(&["ls-files", "-z", "--cached"])?;
        // `ls-files` lists the files relative to the directory it runs in.
        let names = fields(&output).map(str::to_owned).collect();
        Ok(TrackedFiles { worktree: self.worktree.clone(), names })
    }

    /// The files that git sees renamed inside the project root, between the
    /// last commit and the files in the worktree, which in a pre-commit hook
    /// are the commit's renames. There are none outside a repository, before
    /// its first commit, or without git, since following renames is only a
    /// convenience. A file renamed into or out of the root is not a rename
    /// inside it.
    pub fn renames(&self) -> Renames {
        self.find_renames().unwrap_or_else(|error| {
            debug!("not following renamed files: {error}");
            Renames::default()
        })
    }

    /// [`Self::renames`], or why git could not list them.
    fn find_renames(&self) -> Result<Renames, GitError> {
        // 1. Compare the last commit with the files in the worktree, finding renames.
        let output = self.run(&["diff", "-M", "--name-status", "-z", "--no-ext-diff", "HEAD"])?;

        // 2. Git names the files relative to the worktree's root, and the project's are
        //    relative to its own root, which may be a directory inside it.
        let Some(prefix) = relative_posix(&self.worktree, &self.root) else {
            return Ok(Renames::default());
        };
        let inside = |name: &str| match prefix.as_str() {
            "" => Some(name.to_owned()),
            prefix => name.strip_prefix(prefix)?.strip_prefix('/').map(str::to_owned),
        };

        // 3. Each change is its status, then its path, or the old and new paths of a
        //    rename (`R` and its similarity), or of a copy (`C`), which `diff.renames =
        //    "copies"` can turn on.
        let mut names = FxHashMap::default();
        let mut fields = fields(&output);
        while let Some(status) = fields.next() {
            if !status.starts_with(['R', 'C']) {
                fields.next();
                continue;
            }
            let (Some(old), Some(new)) = (fields.next(), fields.next()) else { break };
            if status.starts_with('R')
                && let (Some(old), Some(new)) = (inside(old), inside(new))
            {
                names.insert(old, new);
            }
        }
        Ok(Renames { names })
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

/// The files that git sees renamed inside a project, as [`Git::renames`]
/// finds them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Renames {
    /// The new name of each renamed file, by its old one, both POSIX paths
    /// relative to the project root.
    names: FxHashMap<String, String>,
}

impl Renames {
    /// The new name of the file `name`, both relative to the project root:
    /// its name after the rename, or `name` itself if it wasn't renamed.
    pub fn follow<'n>(&'n self, name: &'n str) -> &'n str {
        self.names.get(name).map_or(name, String::as_str)
    }

    /// The number of renamed files.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether no file was renamed.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Renames from `(old, new)` names, relative to the project root.
impl FromIterator<(String, String)> for Renames {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(renames: I) -> Self {
        Self { names: renames.into_iter().collect() }
    }
}

/// The fields of git's `-z` output, which separates them with NUL bytes, and
/// keeps names as they are, without quoting.
fn fields(output: &str) -> impl Iterator<Item = &str> {
    output.split('\0').filter(|field| !field.is_empty())
}

impl WalkResult {
    /// Keep only the files that git tracks, so that untracked source files
    /// aren't scanned, and untracked packages aren't found.
    pub fn retain_tracked(&mut self, tracked: &TrackedFiles) {
        self.source_files.retain(|path| tracked.contains(path));
        self.package_jsons.retain(|path| tracked.contains(path));
    }
}

/// Why git couldn't tell zenith what it asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// `git` could not be run, most likely because it isn't installed, with
    /// this message.
    Run(String),

    /// Git ran but failed, most likely outside a repository.
    Failed {
        /// The command's arguments, e.g. `ls-files -z --cached`.
        command: String,

        /// What git said.
        message: String,
    },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(error) => write!(f, "could not run `git`: {error}"),
            Self::Failed { command, message } => write!(f, "`git {command}` failed: {message}"),
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

    /// A git repository with `files`, each with contents of its own, so that
    /// git sees them renamed when they move, committed, and its canonical
    /// root.
    fn committed(files: &[&str]) -> (TempDir, PathBuf) {
        let (dir, root) = repository(files, files);
        for file in files {
            fs::write(root.join(file), format!("export const name = '{file}';\n")).unwrap();
        }
        git(&root, &["add", "--all"]);
        let config = ["user.name=zenith", "user.email=zenith@example.com", "commit.gpgSign=false"];
        let mut commit: Vec<_> = config.iter().flat_map(|setting| ["-c", setting]).collect();
        commit.extend(["commit", "--quiet", "--message", "test"]);
        git(&root, &commit);
        (dir, root)
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
        let tracked = Git::new(&root).tracked_files().unwrap();
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
        assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
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

    #[test]
    fn renames_since_the_last_commit_are_found() {
        let (_dir, root) = committed(&["src/a.ts", "src/b.ts", "src/old/c.ts", "d.ts"]);
        git(&root, &["mv", "src/a.ts", "src/z.ts"]);
        git(&root, &["mv", "src/old", "src/new"]);
        fs::write(root.join("src/b.ts"), "export const b = 'changed';\n").unwrap();
        fs::remove_file(root.join("d.ts")).unwrap();

        let renames = Git::new(&root).renames();
        assert_eq!(renames.len(), 2);
        assert_eq!(renames.follow("src/a.ts"), "src/z.ts");
        assert_eq!(renames.follow("src/old/c.ts"), "src/new/c.ts");
        assert_eq!(renames.follow("src/b.ts"), "src/b.ts");
    }

    #[test]
    fn renames_are_named_relative_to_a_root_inside_the_repository() {
        let (_dir, repository) = committed(&["web/a.ts", "web/b.ts", "api/d.ts"]);
        fs::create_dir(repository.join("web/src")).unwrap();
        git(&repository, &["mv", "web/a.ts", "web/src/a.ts"]);
        // Out of the root, and into it.
        git(&repository, &["mv", "web/b.ts", "api/b.ts"]);
        git(&repository, &["mv", "api/d.ts", "web/d.ts"]);

        let root = repository.join("web");
        let git = Git::new(&root).with_env("GIT_INDEX_FILE", ".git/index");
        let renames = git.find_renames().unwrap();
        assert_eq!(renames, [("a.ts".to_string(), "src/a.ts".to_string())].into_iter().collect());
    }

    #[test]
    fn there_are_no_renames_before_the_first_commit_or_outside_a_repository() {
        let (_dir, root) = repository(&["a.ts"], &["a.ts"]);
        assert!(Git::new(&root).find_renames().is_err());
        assert!(Git::new(&root).renames().is_empty());

        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        // A repository above the temporary directory could have renames.
        if !root.ancestors().any(|dir| dir.join(".git").exists()) {
            assert!(Git::new(&root).renames().is_empty());
        }
    }
}
