//! `zenith check --since`, run on a project in a temporary git repository,
//! which the UI snapshot tests can't build.

use std::{fs, path::Path, process::Command};

use serde_json::Value;
use tempfile::TempDir;
use zenith_workspace::testing::{commit, git};

/// Write `contents` to `file` in `root`, creating its directories.
fn write(root: &Path, file: &str, contents: &str) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
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
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(root, "zenith.toml", include_str!("fixtures/since.toml"));
    write(root, "src/api/client.ts", "export const client = 1;\n");
    write(root, "src/web/a.ts", "import { client } from \"../api/client\";\n");
    write(root, "src/web/b.ts", "import { client } from \"../api/client\";\n");
    git(root, &["init", "--quiet"]);
    commit(root);
    write(
        root,
        "src/web/b.ts",
        "import { client } from \"../api/client\";\nexport const b = client;\n",
    );

    assert_eq!(sarif_files(root, &[]), ["src/web/a.ts", "src/web/b.ts"]);
    assert_eq!(sarif_files(root, &["--since", "HEAD"]), ["src/web/b.ts"]);
}
