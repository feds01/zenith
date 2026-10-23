//! What git knows about a project, through a [`Git`] client: the files that
//! it tracks, for scanning only those, as in a pre-commit hook, where
//! untracked files aren't part of the commit, the files that it sees
//! renamed, for following them, the files that changed since a commit, for
//! reporting on only those, and the project at another commit, through a
//! [`Checkout`].

mod checkout;

use std::{
    ffi::OsString,
    fmt, iter,
    path::{Path, PathBuf},
    process::Command,
};

pub use checkout::{Checkout, remove_checkouts};
use log::{debug, warn};
use rustc_hash::{FxHashMap, FxHashSet};
use zenith_diagnostics::message;
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

    /// The files that git tracks in the worktree: those in its index, which
    /// is the next commit's while a pre-commit hook runs.
    pub fn tracked_files(&self) -> Result<TrackedFiles, GitError> {
        self.ensure_repository()?;
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
        self.renames_between("HEAD", None)
    }

    /// The files that git sees renamed inside the project root between the
    /// commit `base` and the commit `head`, or the files in the worktree
    /// without one. Like [`Self::renames`], there are none if git can't list
    /// them.
    pub fn renames_between(&self, base: &str, head: Option<&str>) -> Renames {
        let revisions: Vec<_> = iter::once(base).chain(head).collect();
        self.find_renames(&revisions).unwrap_or_else(|error| {
            debug!("{}", message!(workspace_renames_not_followed, error));
            Renames::default()
        })
    }

    /// The renames between the `revisions` that git compares, or why git
    /// could not list them.
    fn find_renames(&self, revisions: &[&str]) -> Result<Renames, GitError> {
        // 1. Compare the revisions, or a revision with the files in the worktree,
        //    finding renames.
        let mut args = vec!["diff", "-M", "--name-status", "-z", "--no-ext-diff"];
        args.extend(revisions);
        args.push("--");
        let output = self.run(&args)?;

        // 2. Git names the files relative to the worktree's root, and the project's are
        //    relative to its own root, which may be a directory inside it.
        let inside = |name| self.inside(name);

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

    /// The files inside the project root that a branch changed since it left
    /// `since`: those that differ between the merge base of `since` and
    /// `HEAD` and the files in the worktree, as in `git diff <since>...` and
    /// a pull request's diff, so that what `since` gained after the branch
    /// left it is not among them. They are those added, copied, modified,
    /// renamed or deleted, a renamed file by both its old and its new name,
    /// and, if `untracked` is set, the untracked files that git doesn't
    /// ignore. Each is a POSIX path relative to the project root, and they
    /// are sorted. If `since` and `HEAD` share no history, as in a shallow
    /// clone without the merge base, the files are compared with `since`
    /// itself, with a warning.
    ///
    /// `since` is anything that names a commit, as for
    /// [`Self::resolve_commit`].
    pub fn changed_files(&self, since: &str, untracked: bool) -> Result<Vec<String>, GitError> {
        // 1. Find where the branch left the commit. Before the first commit, `HEAD`
        //    names none.
        let base = self.branch_point(since, "HEAD")?;

        // 2. Compare it with the files in the worktree, without finding renames, so
        //    that a renamed file is deleted under its old name and added under its new
        //    one.
        let diff =
            self.run(&["diff", "--name-only", "--no-renames", "-z", "--no-ext-diff", &base])?;

        // 3. Add the untracked files, which git doesn't compare.
        let others = match untracked {
            true => self.run(&["ls-files", "-z", "--others", "--exclude-standard"])?,
            false => String::new(),
        };

        // 4. Keep those inside the project root, named relative to it.
        let mut names: Vec<_> =
            fields(&diff).chain(fields(&others)).filter_map(|name| self.inside(name)).collect();
        names.sort_unstable();
        names.dedup();
        Ok(names)
    }

    /// The hash of the commit that `reference` names: anything that names a
    /// commit, such as a branch, a tag, `HEAD~1` or a hash. One that names
    /// none is [`GitError::UnknownCommit`], and outside a repository, it is
    /// [`GitError::NotARepository`].
    pub fn resolve_commit(&self, reference: &str) -> Result<String, GitError> {
        self.ensure_repository()?;
        let revision = format!("{reference}^{{commit}}");
        // A reference such as `--git-dir` is a revision, not an option.
        match self.run(&["rev-parse", "--verify", "--quiet", "--end-of-options", &revision]) {
            Ok(hash) => Ok(hash.trim().to_owned()),
            // `--quiet` silences only the error that no commit has that name.
            Err(GitError::Failed { message, .. }) if message.is_empty() => {
                Err(GitError::UnknownCommit(reference.to_owned()))
            }
            Err(error) => Err(error),
        }
    }

    /// The hash of the commit where the branch at `head` left `reference`:
    /// their merge base, as in `git diff <reference>...<head>` and a pull
    /// request's diff. If they share no history, as in a shallow clone
    /// without the merge base, it is the commit of `reference` itself, with a
    /// warning. Both are anything that names a commit, as for
    /// [`Self::resolve_commit`], which finds `reference` first, so that an
    /// unknown one has an error of its own.
    pub fn branch_point(&self, reference: &str, head: &str) -> Result<String, GitError> {
        let commit = self.resolve_commit(reference)?;
        let head_commit = self.resolve_commit(head)?;
        match self.merge_base(&commit, &head_commit)? {
            Some(base) => Ok(base),
            None => {
                warn!("{}", message!(workspace_no_merge_base, reference, head));
                Ok(commit)
            }
        }
    }

    /// The hash of the best common ancestor of the commits `first` and
    /// `second`, where the history of one left the other, or `None` if they
    /// share no history.
    pub fn merge_base(&self, first: &str, second: &str) -> Result<Option<String>, GitError> {
        match self.run(&["merge-base", "--end-of-options", first, second]) {
            Ok(hash) => Ok(Some(hash.trim().to_owned())),
            // Git fails without a word when there is none.
            Err(GitError::Failed { message, .. }) if message.is_empty() => Ok(None),
            Err(error) => Err(error),
        }
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
        self.manifests.retain(|path| tracked.contains(path));
    }
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
    use std::{fs, sync::Mutex};

    use log::{Level, LevelFilter, Log, Metadata, Record};
    use tempfile::TempDir;
    use zenith_utils::path::adjust_canonicalisation;

    use super::*;
    use crate::testing::{commit, git};

    /// The warnings logged while the tests run, on any thread.
    static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// A logger that keeps the warnings in [`WARNINGS`].
    struct Warnings;

    impl Log for Warnings {
        fn enabled(&self, metadata: &Metadata) -> bool {
            metadata.level() <= Level::Warn
        }

        fn log(&self, record: &Record) {
            if self.enabled(record.metadata()) {
                WARNINGS.lock().unwrap().push(record.args().to_string());
            }
        }

        fn flush(&self) {}
    }

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
    fn renames_between_commits_are_found() {
        let (_dir, root) = committed(&["src/a.ts", "src/b.ts"]);
        git(&root, &["mv", "src/a.ts", "src/z.ts"]);
        commit(&root);
        git(&root, &["mv", "src/b.ts", "src/y.ts"]);

        let git = Git::new(&root);
        let between = git.renames_between("HEAD~1", Some("HEAD"));
        assert_eq!(
            between,
            [("src/a.ts".to_string(), "src/z.ts".to_string())].into_iter().collect()
        );
        let since = git.renames_between("HEAD~1", None);
        assert_eq!(since.len(), 2);
        assert_eq!(since.follow("src/b.ts"), "src/y.ts");
        assert!(git.renames_between("no-such-branch", None).is_empty());
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
        let renames = git.find_renames(&["HEAD"]).unwrap();
        assert_eq!(renames, [("a.ts".to_string(), "src/a.ts".to_string())].into_iter().collect());
    }

    #[test]
    fn there_are_no_renames_before_the_first_commit_or_outside_a_repository() {
        let (_dir, root) = repository(&["a.ts"], &["a.ts"]);
        assert!(Git::new(&root).find_renames(&["HEAD"]).is_err());
        assert!(Git::new(&root).renames().is_empty());

        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        // A repository above the temporary directory could have renames.
        if !root.ancestors().any(|dir| dir.join(".git").exists()) {
            assert!(Git::new(&root).renames().is_empty());
        }
    }

    #[test]
    fn changed_files_are_those_added_modified_renamed_or_deleted() {
        let (_dir, root) = committed(&["src/a.ts", "src/b.ts", "src/c.ts", "src/d.ts"]);
        fs::write(root.join("src/a.ts"), "export const a = 'changed';\n").unwrap();
        git(&root, &["mv", "src/b.ts", "src/z.ts"]);
        fs::remove_file(root.join("src/c.ts")).unwrap();
        fs::write(root.join("src/new.ts"), "").unwrap();
        fs::write(root.join(".gitignore"), "ignored.ts\n").unwrap();
        fs::write(root.join("ignored.ts"), "").unwrap();

        let git = Git::new(&root);
        let tracked = ["src/a.ts", "src/b.ts", "src/c.ts", "src/z.ts"];
        assert_eq!(git.changed_files("HEAD", false).unwrap(), tracked);
        let untracked =
            [".gitignore", "src/a.ts", "src/b.ts", "src/c.ts", "src/new.ts", "src/z.ts"];
        assert_eq!(git.changed_files("HEAD", true).unwrap(), untracked);
    }

    #[test]
    fn changed_files_are_found_since_a_branch_a_hash_or_an_ancestor() {
        let (_dir, root) = committed(&["a.ts", "b.ts"]);
        git(&root, &["branch", "base"]);
        fs::write(root.join("a.ts"), "export const a = 'changed';\n").unwrap();
        commit(&root);
        fs::write(root.join("b.ts"), "export const b = 'changed';\n").unwrap();

        let git = Git::new(&root);
        let hash = git.resolve_commit("base").unwrap();
        for since in ["base", hash.as_str(), "HEAD~1"] {
            assert_eq!(git.changed_files(since, true).unwrap(), ["a.ts", "b.ts"], "{since}");
        }
        assert_eq!(git.changed_files("HEAD", true).unwrap(), ["b.ts"]);
    }

    #[test]
    fn changed_files_leave_out_what_the_other_branch_gained_after_the_branch_left_it() {
        let (_dir, root) = committed(&["a.ts", "b.ts", "c.ts"]);
        git(&root, &["branch", "trunk"]);
        git(&root, &["checkout", "--quiet", "-b", "feature"]);
        fs::write(root.join("b.ts"), "export const b = 'changed';\n").unwrap();
        commit(&root);
        git(&root, &["checkout", "--quiet", "trunk"]);
        fs::write(root.join("a.ts"), "export const a = 'changed';\n").unwrap();
        commit(&root);
        git(&root, &["checkout", "--quiet", "feature"]);
        fs::write(root.join("c.ts"), "export const c = 'changed';\n").unwrap();

        assert_eq!(Git::new(&root).changed_files("trunk", true).unwrap(), ["b.ts", "c.ts"]);
    }

    #[test]
    fn changed_files_are_compared_with_a_commit_without_shared_history_with_a_warning() {
        // A process has one logger, so only this test may install it.
        log::set_logger(&Warnings).expect("only this test installs a logger");
        log::set_max_level(LevelFilter::Warn);
        let (_dir, root) = committed(&["a.ts", "b.ts"]);
        git(&root, &["branch", "unrelated"]);
        git(&root, &["checkout", "--quiet", "--orphan", "fresh"]);
        fs::write(root.join("c.ts"), "export const c = 'new';\n").unwrap();
        commit(&root);
        fs::write(root.join("b.ts"), "export const b = 'changed';\n").unwrap();

        assert_eq!(Git::new(&root).changed_files("unrelated", true).unwrap(), ["b.ts", "c.ts"]);
        let warning = "`unrelated` shares no history with `HEAD` here, so the files are compared \
                       with its tip; in a shallow clone, fetch more history";
        assert!(WARNINGS.lock().unwrap().iter().any(|logged| logged == warning));
    }

    #[test]
    fn changed_files_are_named_relative_to_a_root_inside_the_repository() {
        let (_dir, repository) = committed(&["web/a.ts", "api/b.ts"]);
        fs::write(repository.join("web/a.ts"), "export const a = 'changed';\n").unwrap();
        fs::write(repository.join("api/b.ts"), "export const b = 'changed';\n").unwrap();
        // Out of the root.
        git(&repository, &["mv", "web/a.ts", "api/a.ts"]);

        let root = repository.join("web");
        assert_eq!(Git::new(&root).changed_files("HEAD", true).unwrap(), ["a.ts"]);
    }

    #[test]
    fn an_unknown_commit_or_a_missing_repository_is_an_error_of_its_own() {
        let (_dir, root) = committed(&["a.ts"]);
        let error = Git::new(&root).changed_files("no-such-branch", true).unwrap_err();
        assert_eq!(error, GitError::UnknownCommit("no-such-branch".into()));

        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        // A repository above the temporary directory would know its commits.
        if !root.ancestors().any(|dir| dir.join(".git").exists()) {
            let error = Git::new(&root).changed_files("HEAD", true).unwrap_err();
            assert_eq!(error, GitError::NotARepository(root));
        }
    }

    #[test]
    fn a_reference_like_an_option_is_a_commit_name() {
        let (_dir, root) = committed(&["a.ts"]);
        let error = Git::new(&root).resolve_commit("--git-dir").unwrap_err();
        assert_eq!(error, GitError::UnknownCommit("--git-dir".into()));
    }

    #[test]
    fn before_the_first_commit_head_is_an_unknown_commit() {
        let (_dir, root) = repository(&["a.ts"], &["a.ts"]);
        let error = Git::new(&root).changed_files("HEAD", true).unwrap_err();
        assert_eq!(error, GitError::UnknownCommit("HEAD".into()));

        // On a branch without a commit, beside one with commits.
        let (_dir, root) = committed(&["a.ts"]);
        git(&root, &["branch", "trunk"]);
        git(&root, &["checkout", "--quiet", "--orphan", "fresh"]);
        let error = Git::new(&root).changed_files("trunk", true).unwrap_err();
        assert_eq!(error, GitError::UnknownCommit("HEAD".into()));
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
