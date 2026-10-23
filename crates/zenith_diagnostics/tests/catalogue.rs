//! Tests of the catalogue itself, which the macros can't check: each
//! macro sees only the messages that its own code names.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// The directory of the crates.
fn crates() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

/// The paths of the files under `dir` whose extension is `extension`.
fn files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && path.file_name().is_some_and(|name| name != "target") {
            files.extend(self::files(&path, extension));
        } else if path.extension().is_some_and(|found| found == extension) {
            files.push(path);
        }
    }
    files
}

/// The ids of the messages in `text`, a Fluent file, in order.
fn ids(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .filter(|line| line.starts_with(|c: char| c.is_ascii_lowercase()))
        .filter_map(|line| line.split_once(" =").map(|(id, _)| id))
}

#[test]
fn the_macros_read_every_file_of_the_catalogue() {
    let read: BTreeSet<_> = zenith_macros::catalogue_sources!()
        .iter()
        .map(|(file, _)| file.to_string())
        .filter(|file| file != "tests.ftl")
        .collect();
    let messages = crates().join("zenith_diagnostics/messages/en-US");
    let on_disk: BTreeSet<_> = files(&messages, "ftl")
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(read, on_disk, "list each file of the catalogue in zenith_macros");
}

#[test]
fn every_message_is_named_by_code_or_by_a_message_that_is() {
    // 1. The code, without comments or blanks, in which a message is named
    // as `diag(id)`, `message!(id` or as an argument of a macro.
    let code: String = files(crates(), "rs")
        .iter()
        .flat_map(|path| {
            let text = fs::read_to_string(path).unwrap();
            let lines: Vec<String> = text
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .map(|line| line.split_whitespace().collect())
                .collect();
            lines
        })
        .collect();
    let named_in_code = |id: &str| {
        ["diag(", "message!("].iter().any(|start| {
            [")", ",", "."].iter().any(|end| code.contains(&format!("{start}{id}{end}")))
        })
    };

    // 2. The messages, and those that each refers to.
    let catalogue: Vec<String> = files(&crates().join("zenith_diagnostics"), "ftl")
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    let mut messages = Vec::new();
    for text in &catalogue {
        let starts: Vec<_> = ids(text).filter_map(|id| text.find(&format!("\n{id} ="))).collect();
        for (index, id) in ids(text).enumerate() {
            let end = starts.get(index + 1).copied().unwrap_or(text.len());
            let body = &text[starts.get(index).copied().unwrap_or(0)..end];
            messages.push((id.to_string(), body.to_string()));
        }
    }

    // 3. Those named by code are used, and so are those that a used message
    // refers to, until no more are.
    let mut used: BTreeSet<String> =
        messages.iter().filter(|(id, _)| named_in_code(id)).map(|(id, _)| id.clone()).collect();
    loop {
        let referred: Vec<String> = messages
            .iter()
            .filter(|(id, _)| !used.contains(id))
            .filter(|(id, _)| {
                messages
                    .iter()
                    .any(|(user, body)| used.contains(user) && body.contains(&format!("{{{id}}}")))
            })
            .map(|(id, _)| id.clone())
            .collect();
        if referred.is_empty() {
            break;
        }
        used.extend(referred);
    }
    let unused: Vec<_> = messages.iter().filter(|(id, _)| !used.contains(id)).collect();
    assert!(unused.is_empty(), "messages that nothing names: {unused:#?}");
}
