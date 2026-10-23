//! `zenith check --since`, run on a project in a temporary git repository,
//! which the UI snapshot tests can't build.

use std::{fs, path::Path, process::Command};

use serde_json::Value;
use tempfile::TempDir;
use zenith_workspace::testing::{commit, git};

/// Copy the files of `from` into `to`, leaving out the UI test's own.
fn copy_project(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_project(&path, &target);
        } else if !path.ends_with("args") && !path.ends_with("output.snap") {
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// The files of the results of `zenith check` with `args`, in `root`, as a
/// SARIF log names them.
fn sarif_files(root: &Path, args: &[&str]) -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_zenith"))
        .current_dir(root)
        .args(["check", "--format", "sarif"])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    let sarif: Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    let uri = |result: &Value| {
        let location = &result["locations"][0]["physicalLocation"]["artifactLocation"];
        location["uri"].as_str().unwrap().to_owned()
    };
    results.iter().map(uri).collect()
}

#[test]
fn since_narrows_the_sarif_results_to_the_changed_files() {
    // Two files import `src/api`, which they mustn't; only one changes.
    let case = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cases/features/check-paths");
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    copy_project(&case, root);
    git(root, &["init", "--quiet"]);
    commit(root);
    let b = root.join("src/web/b.ts");
    fs::write(&b, fs::read_to_string(&b).unwrap() + "export const changed = true;\n").unwrap();

    assert_eq!(sarif_files(root, &[]), ["src/web/a.ts", "src/web/b.ts"]);
    assert_eq!(sarif_files(root, &["--since", "HEAD"]), ["src/web/b.ts"]);
}
