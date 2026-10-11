//! UI snapshot tests: run the `zenith` binary on the projects in
//! `tests/cases` and compare its exit codes and output with the case's
//! `output.snap`.
//!
//! A case is a directory with an `args` file or a `zenith.toml`. Besides the
//! project's own files, it has:
//!
//! - `args`: one `zenith` invocation per line, `check` by default. Blank lines
//!   and lines starting with `#` are skipped, and double quotes group an
//!   argument. Every invocation gets `--color never`.
//! - `fixture`: optionally, the name of a project in `tests/fixtures` to copy
//!   first. The case's own files are copied over it.
//! - `output.snap`: the exit code, `stdout` and `stderr` of every invocation.
//!
//! Each case runs as its own test, in a copy of the project in a temporary
//! directory outside the repository, so the repository's `.gitignore` can't
//! affect it. The project's path is shown as `$ROOT`, with `/` separators.
//! Run `just update-snapshots` (`REGENERATE_OUTPUT=1`) to re-record them.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use libtest_mimic::{Arguments, Failed, Trial};

/// The files of a case that are not part of its project.
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

/// Add the cases in `dir` and its subdirectories to `cases`.
fn find_cases(dir: &Path, cases: &mut Vec<PathBuf>) {
    if dir.join("args").is_file() || dir.join("zenith.toml").is_file() {
        cases.push(dir.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            find_cases(&entry.path(), cases);
        }
    }
}

/// Run the invocations of the case in `case`, and compare their output with
/// its snapshot, or record the snapshot.
fn run_case(tests_dir: &Path, case: &Path) -> Result<(), Failed> {
    let project = temp_project();
    if let Ok(fixture) = fs::read_to_string(case.join("fixture")) {
        let fixture = tests_dir.join("fixtures").join(fixture.trim());
        copy_dir(&fixture, &project, &[]).map_err(|e| format!("copying the fixture: {e}"))?;
    }
    copy_dir(case, &project, CASE_FILES).map_err(|e| format!("copying the case: {e}"))?;

    let invocations = fs::read_to_string(case.join("args")).unwrap_or_else(|_| "check".into());
    let mut output = String::new();
    for line in invocations.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        output += &run_invocation(&project, line)?;
    }
    let _ = fs::remove_dir_all(&project);

    let snapshot = case.join("output.snap");
    let regenerate = env::var_os("REGENERATE_OUTPUT").is_some_and(|v| !v.is_empty() && v != "0");
    match fs::read_to_string(&snapshot) {
        Ok(expected) if !regenerate => {
            // Git may check the snapshot out with Windows line endings.
            pretty_assertions::assert_str_eq!(expected.replace("\r\n", "\n"), output);
            Ok(())
        }
        _ => fs::write(&snapshot, &output).map_err(|e| format!("writing the snapshot: {e}").into()),
    }
}

/// Run `zenith` with the arguments on `line` in `project`, and record its
/// exit code and output.
fn run_invocation(project: &Path, line: &str) -> Result<String, Failed> {
    let result = Command::new(env!("CARGO_BIN_EXE_zenith"))
        .args(split_args(line))
        .args(["--color", "never"])
        .current_dir(project)
        .env_remove("NO_COLOR")
        .output()
        .map_err(|e| format!("running zenith: {e}"))?;

    let root = project.to_string_lossy();
    let normalise = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
        let mut text = text.replace(root.as_ref(), "$ROOT").replace('\\', "/");
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text
    };

    let mut output = format!("$ zenith {line}\nexit: {}\n", result.status.code().unwrap_or(-1));
    for (name, stream) in [("stdout", &result.stdout), ("stderr", &result.stderr)] {
        let text = normalise(stream);
        if !text.is_empty() {
            output += &format!("--- {name}\n{text}");
        }
    }
    Ok(output + "\n")
}

/// Split a line into arguments at whitespace. Double quotes group an
/// argument, and may make an empty one.
fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut started) = (false, false);
    for c in line.chars() {
        match c {
            '"' => (quoted, started) = (!quoted, true),
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// A new, empty and canonical temporary directory.
fn temp_project() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let count = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = env::temp_dir().join(format!("zenith-ui-{}-{count}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // `/tmp` is a symlink on macOS. Windows paths lose their `\\?\` prefix, as
    // zenith's do.
    let dir = fs::canonicalize(&dir).unwrap();
    PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\"))
}

/// Copy the directory `from` into `to`, leaving out the entries named in
/// `skip` at its top level.
fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> io::Result<()> {
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
