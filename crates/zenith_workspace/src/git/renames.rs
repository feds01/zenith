//! The files that git sees renamed, for following them.

use std::iter;

use log::debug;
use rustc_hash::FxHashMap;
use zenith_diagnostics::message;

use super::{Git, GitError, fields};

impl Git {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::fs::adjust_canonicalisation;

    use super::*;
    use crate::{
        git::tests::{committed, repository},
        testing::{commit, git},
    };

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
}
