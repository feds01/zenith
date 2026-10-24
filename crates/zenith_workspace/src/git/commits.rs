//! The commits that references name, where a branch left another, and the
//! files that changed since a commit, for reporting on only those.

use log::warn;
use zenith_diagnostics::message;

use super::{Git, GitError, fields};

impl Git {
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
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Mutex};

    use log::{Level, LevelFilter, Log, Metadata, Record};
    use tempfile::TempDir;
    use zenith_utils::path::adjust_canonicalisation;

    use super::*;
    use crate::{
        git::tests::{committed, repository},
        testing::{commit, git},
    };

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
}
