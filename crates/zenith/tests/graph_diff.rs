//! `zenith graph diff` on a git repository in a temporary directory: the
//! dependencies it finds added and removed between commits, and the working
//! tree, and the temporary checkouts it leaves behind, which must be none.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

use pretty_assertions::assert_eq;
use tempfile::TempDir;
use zenith_workspace::testing;

/// A configuration that leaves `src/legacy.ts` out.
const CONFIG: &str = include_str!("fixtures/graph-diff.toml");

/// A git repository, and the directory that `zenith` makes its temporary
/// checkouts in, which must be empty after every run.
struct Repository {
    /// The temporary directory, which holds both.
    _dir: TempDir,

    /// The repository's root.
    root: PathBuf,

    /// The temporary directory that `zenith` is given.
    scratch: PathBuf,
}

impl Repository {
    /// A new, empty repository.
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let (root, scratch) = (dir.path().join("repository"), dir.path().join("scratch"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(&scratch).unwrap();
        testing::git(&root, &["init", "--quiet"]);
        Self { _dir: dir, root, scratch }
    }

    /// Write `files`, each a path and its contents, removing those with
    /// none.
    fn write(&self, files: &[(&str, Option<&str>)]) {
        for (file, contents) in files {
            let path = self.root.join(file);
            match contents {
                Some(contents) => {
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(path, contents).unwrap();
                }
                None => fs::remove_file(path).unwrap(),
            }
        }
    }

    /// Commit every change.
    fn commit(&self) {
        testing::commit(&self.root);
    }

    /// Run git with `args`.
    fn git(&self, args: &[&str]) {
        testing::git(&self.root, args);
    }

    /// What git prints when run with `args`.
    fn git_output(&self, args: &[&str]) -> String {
        testing::git_output(&self.root, args)
    }

    /// Run `zenith` with `args` in the repository, outside any hook that
    /// could have set git's variables, checking that it leaves no temporary
    /// checkout behind.
    fn zenith(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_zenith"));
        // Options before the command, since those after `--` are a selector.
        command.current_dir(&self.root).args(["--no-cache", "--color", "never"]).args(args);
        for variable in ["TMPDIR", "TMP", "TEMP"] {
            command.env(variable, &self.scratch);
        }
        for variable in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
            command.env_remove(variable);
        }
        let output = command.output().unwrap();
        let left: Vec<_> = fs::read_dir(&self.scratch).unwrap().collect();
        assert!(left.is_empty(), "zenith {args:?} left {left:?} behind");
        output
    }
}

/// What `output` printed on `stream`, with Windows line endings normalised.
fn text(stream: &[u8]) -> String {
    String::from_utf8_lossy(stream).replace("\r\n", "\n")
}

/// A repository whose `src/app.ts` imports `lib/a.ts` and `util.ts`, and whose
/// `lib/a.ts` imports `types.ts`, committed, then changed so that `app.ts`
/// imports `types.ts` too, `lib/a.ts` no longer does, and `util.ts` is
/// renamed to `helpers.ts`.
fn changed_repository() -> Repository {
    let repository = Repository::new();
    let util = "export const util = 'a utility that keeps its contents when it moves';\n";
    repository.write(&[
        ("src/app.ts", Some("import './lib/a';\nimport './util';\n")),
        ("src/lib/a.ts", Some("import '../types';\n")),
        ("src/types.ts", Some("export type T = string;\n")),
        ("src/util.ts", Some(util)),
    ]);
    repository.commit();

    repository.git(&["mv", "src/util.ts", "src/helpers.ts"]);
    repository.write(&[
        ("src/app.ts", Some("import './lib/a';\nimport './types';\nimport './helpers';\n")),
        ("src/lib/a.ts", Some("export const a = 1;\n")),
    ]);
    repository
}

/// The diff of `src` from the commit to the working tree, or to a commit of
/// the changes.
const SRC_DIFF: &str = "\
dependencies between the children of `src` that changed:
  + src/app.ts -> src/types.ts (1 import)
  - src/lib -> src/types.ts (1 import)
";

#[test]
fn finds_the_dependencies_added_and_removed_since_a_commit() {
    let repository = changed_repository();
    let output = repository.zenith(&["graph", "diff", "HEAD", "--", "src"]);
    assert_eq!(text(&output.stderr), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(&output.stdout), SRC_DIFF);

    // The checkout changes neither the repository's index nor its worktrees.
    assert_eq!(
        repository.git_output(&["worktree", "list", "--porcelain"]).matches("worktree ").count(),
        1
    );
    assert_eq!(
        repository.git_output(&["diff", "--cached", "--name-status"]),
        "R100\tsrc/util.ts\tsrc/helpers.ts\n"
    );
}

#[test]
fn finds_the_dependencies_added_and_removed_between_commits() {
    let repository = changed_repository();
    repository.commit();
    let output = repository.zenith(&["graph", "diff", "HEAD~1", "HEAD", "--", "src"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(&output.stdout), SRC_DIFF);

    let output =
        repository.zenith(&["graph", "diff", "HEAD~1", "HEAD", "--format", "json", "--", "src"]);
    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{
            "module": "src",
            "added": [{ "from": "src/app.ts", "to": "src/types.ts", "imports": 1 }],
            "removed": [{ "from": "src/lib", "to": "src/types.ts", "imports": 1 }],
        }])
    );

    // Nothing changed from a commit to itself.
    let output = repository.zenith(&["graph", "diff", "HEAD", "HEAD"]);
    assert_eq!(text(&output.stdout), "no dependencies between the children of `.` changed\n");
}

#[test]
fn compares_with_where_the_branch_left_the_base() {
    let repository = Repository::new();
    repository.write(&[
        ("src/app.ts", Some("import './a';\n")),
        ("src/a.ts", Some("export const a = 1;\n")),
        ("src/b.ts", Some("export const b = 1;\n")),
    ]);
    repository.commit();

    // The base gains a dependency after the branch leaves it, which is not the
    // branch's change.
    repository.git(&["checkout", "--quiet", "-b", "trunk"]);
    repository.write(&[("src/app.ts", Some("import './a';\nimport './b';\n"))]);
    repository.commit();
    repository.git(&["checkout", "--quiet", "-"]);
    repository.write(&[("src/a.ts", Some("import './b';\n"))]);

    let output = repository.zenith(&["graph", "diff", "trunk", "--", "src"]);
    assert_eq!(output.status.code(), Some(0));
    let expected = "\
dependencies between the children of `src` that changed:
  + src/a.ts -> src/b.ts (1 import)
";
    assert_eq!(text(&output.stdout), expected);
}

#[test]
fn compares_the_nodes_further_below_the_selector() {
    let repository = changed_repository();
    let output = repository.zenith(&["graph", "diff", "HEAD", "--depth", "2", "--", "src"]);
    assert_eq!(output.status.code(), Some(0));
    let expected = "\
dependencies between the nodes 2 levels below `src` that changed:
  + src/app.ts -> src/types.ts (1 import)
  - src/lib/a.ts -> src/types.ts (1 import)
";
    assert_eq!(text(&output.stdout), expected);
}

#[test]
fn leaves_out_the_imports_of_excluded_kinds() {
    let repository = Repository::new();
    repository.write(&[
        ("src/app.ts", Some("import './a';\n")),
        ("src/a.ts", Some("export const a = 1;\n")),
        ("src/types.ts", Some("export type T = string;\n")),
    ]);
    repository.commit();
    repository.write(&[("src/app.ts", Some("import './a';\nimport type { T } from './types';\n"))]);

    let output =
        repository.zenith(&["graph", "diff", "HEAD", "--exclude-kind", "type", "--", "src"]);
    assert_eq!(text(&output.stdout), "no dependencies between the children of `src` changed\n");
    let output = repository.zenith(&["graph", "diff", "HEAD", "--", "src"]);
    assert!(text(&output.stdout).contains("+ src/app.ts -> src/types.ts (1 import)"));
}

/// A project in `web` whose `src/legacy.ts` imports `types.ts`, committed,
/// then changed so that `app.ts` imports `types.ts` too.
fn web_repository(config_at_base: bool) -> Repository {
    let repository = Repository::new();
    repository.write(&[
        ("web/src/app.ts", Some("import './a';\n")),
        ("web/src/a.ts", Some("export const a = 1;\n")),
        ("web/src/legacy.ts", Some("import './types';\n")),
        ("web/src/types.ts", Some("export type T = string;\n")),
    ]);
    if config_at_base {
        repository.write(&[("zenith.toml", Some(CONFIG))]);
    }
    repository.commit();
    repository.write(&[
        ("zenith.toml", Some(CONFIG)),
        ("web/src/app.ts", Some("import './a';\nimport './types';\n")),
    ]);
    repository
}

#[test]
fn builds_the_base_with_its_root_and_configuration_at_the_same_paths() {
    let repository = web_repository(true);
    let config = repository.root.join("zenith.toml");
    let config = config.to_str().unwrap();
    let args = ["--root", "web", "--config", config, "graph", "diff", "HEAD", "--", "src"];
    let output = repository.zenith(&args);
    assert_eq!(text(&output.stderr), "");
    // `legacy.ts` is left out of both graphs.
    let expected = "\
dependencies between the children of `src` that changed:
  + src/app.ts -> src/types.ts (1 import)
";
    assert_eq!(text(&output.stdout), expected);
}

#[test]
fn builds_the_base_with_the_defaults_without_its_configuration() {
    let repository = web_repository(false);
    let config = repository.root.join("zenith.toml");
    let config = config.to_str().unwrap();
    let args = ["--root", "web", "--config", config, "graph", "diff", "HEAD", "--", "src"];
    let output = repository.zenith(&args);
    assert_eq!(output.status.code(), Some(0));
    // Only the base's graph has `legacy.ts`.
    let expected = "\
dependencies between the children of `src` that changed:
  + src/app.ts -> src/types.ts (1 import)
  - src/legacy.ts -> src/types.ts (1 import)
";
    assert_eq!(text(&output.stdout), expected);
}

#[test]
fn reports_on_the_files_of_the_head_only() {
    let repository = Repository::new();
    repository.write(&[
        ("src/app.ts", Some("import './gone';\nimport './lost';\n")),
        ("src/broken.ts", Some("export const = ;\n")),
    ]);
    repository.commit();
    repository.write(&[
        ("src/app.ts", Some("import './absent';\n")),
        ("src/broken.ts", Some("export const fixed = 1;\n")),
    ]);

    let output = repository.zenith(&["graph", "diff", "HEAD"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        text(&output.stderr),
        "warning: 1 import could not be resolved; run with `-v` to list them\n"
    );
}

#[test]
fn an_unknown_ref_is_an_error() {
    let repository = changed_repository();
    let output = repository.zenith(&["graph", "diff", "no-such-branch"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), "");
    assert_eq!(
        text(&output.stderr),
        "error: git knows no commit named `no-such-branch`; in a shallow clone, such as a CI \
         checkout, fetch it first\n"
    );
}

#[test]
fn a_selector_that_matches_nowhere_is_an_error() {
    let repository = changed_repository();
    let output = repository.zenith(&["graph", "diff", "HEAD", "--", "api"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stderr), "error: `api` does not match anything\n");
}

#[test]
fn the_checkout_is_removed_when_its_graph_cannot_be_built() {
    // The first commit has no source files, and no `web` directory.
    let repository = Repository::new();
    repository.write(&[("README.md", Some("# A project\n"))]);
    repository.commit();
    repository.write(&[("web/src/app.ts", Some("export const app = 1;\n"))]);

    // The errors name the ref, not the temporary directory.
    let output = repository.zenith(&["graph", "diff", "HEAD"]);
    assert_eq!(output.status.code(), Some(2));
    let expected = "error: found no source files in the project root at `HEAD`\n";
    assert_eq!(text(&output.stderr), expected);

    let output = repository.zenith(&["--root", "web", "graph", "diff", "HEAD"]);
    assert_eq!(output.status.code(), Some(2));
    let expected = "error: the project root is not a directory at `HEAD`\n";
    assert_eq!(text(&output.stderr), expected);
}
