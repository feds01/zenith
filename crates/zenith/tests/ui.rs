//! UI snapshot tests: run the `zenith` binary on the projects in
//! `tests/cases/**` and compare its exit code and output with the case's
//! `output.snap`.
//!
//! A case is any directory containing an `args` file or a `zenith.toml`:
//!
//! - `args`: one `zenith` invocation per line (default: `check`). Every
//!   invocation gets `--color never`.
//! - `fixture`: optionally, the name of a project in `tests/fixtures/` to copy
//!   first; the case's own files are copied over it.
//! - `output.snap`: the expected output of every invocation.
//!
//! The project is copied into a temporary directory outside the repository,
//! so that the repository's own `.gitignore` cannot affect the results.
//! Run `just update-snapshots` (`REGENERATE_OUTPUT=1`) to re-record them.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use libtest_mimic::{Arguments, Failed, Trial};

/// Files of a case that are not part of the project.
const CASE_FILES: &[&str] = &["args", "fixture", "output.snap"];

fn main() {
    let args = Arguments::from_args();
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests");
    let cases_dir = tests_dir.join("cases");

    let mut cases = Vec::new();
    find_cases(&cases_dir, &mut cases);
    cases.sort();

    let trials = cases
        .into_iter()
        .map(|case| {
            let name = case.strip_prefix(&cases_dir).unwrap().to_string_lossy().replace('\\', "/");
            let tests_dir = tests_dir.clone();
            Trial::test(name, move || run_case(&tests_dir, &case))
        })
        .collect();

    libtest_mimic::run(&args, trials).exit();
}

fn find_cases(dir: &Path, cases: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    if dir.join("args").is_file() || dir.join("zenith.toml").is_file() {
        cases.push(dir.to_path_buf());
        return;
    }
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            find_cases(&entry.path(), cases);
        }
    }
}

fn run_case(tests_dir: &Path, case: &Path) -> Result<(), Failed> {
    let project = temp_project();

    if let Ok(fixture) = fs::read_to_string(case.join("fixture")) {
        let fixture = tests_dir.join("fixtures").join(fixture.trim());
        copy_dir(&fixture, &project, &[]).map_err(|e| format!("copying fixture: {e}"))?;
    }
    copy_dir(case, &project, CASE_FILES).map_err(|e| format!("copying case: {e}"))?;

    let invocations = fs::read_to_string(case.join("args")).unwrap_or_else(|_| "check".to_string());
    let mut output = String::new();
    for line in
        invocations.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let args = split_args(line);
        let result = Command::new(env!("CARGO_BIN_EXE_zenith"))
            .args(&args)
            .args(["--color", "never"])
            .current_dir(&project)
            .env_remove("ZENITH_CONFIG")
            .env("NO_COLOR", "1")
            .output()
            .map_err(|e| format!("running zenith: {e}"))?;

        let normalise = |bytes: &[u8]| {
            let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
            let root = project.to_string_lossy().into_owned();
            text.replace(&root, "$ROOT").replace('\\', "/")
        };
        output
            .push_str(&format!("$ zenith {line}\nexit: {}\n", result.status.code().unwrap_or(-1)));
        let stdout = normalise(&result.stdout);
        if !stdout.is_empty() {
            output.push_str(&format!("--- stdout\n{}", ensure_newline(stdout)));
        }
        let stderr = normalise(&result.stderr);
        if !stderr.is_empty() {
            output.push_str(&format!("--- stderr\n{}", ensure_newline(stderr)));
        }
        output.push('\n');
    }
    let _ = fs::remove_dir_all(&project);

    let snapshot = case.join("output.snap");
    let regenerate =
        std::env::var_os("REGENERATE_OUTPUT").is_some_and(|v| !v.is_empty() && v != "0");
    match fs::read_to_string(&snapshot) {
        Ok(expected) if !regenerate => {
            let expected = expected.replace("\r\n", "\n");
            if expected != output {
                pretty_assertions::assert_str_eq!(expected, output);
            }
            Ok(())
        }
        _ => {
            fs::write(&snapshot, &output).map_err(|e| format!("writing snapshot: {e}"))?;
            Ok(())
        }
    }
}

fn ensure_newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Split a line into arguments, honouring double quotes.
fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut has_arg = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has_arg = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has_arg {
                    args.push(std::mem::take(&mut current));
                    has_arg = false;
                }
            }
            c => {
                current.push(c);
                has_arg = true;
            }
        }
    }
    if has_arg {
        args.push(current);
    }
    args
}

/// A fresh, canonical temporary directory.
fn temp_project() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "zenith-ui-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // Canonicalise (e.g. `/tmp` is a symlink on macOS) without `\\?\`.
    let dir = fs::canonicalize(&dir).unwrap();
    PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\"))
}

fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|skip| name == *skip) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target, &[])?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
