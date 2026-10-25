//! Tests of which files only tests compile, on the workspace of
//! [`Project::workspace`].

use std::path::{Path, PathBuf};

use thin_vec::ThinVec;
use zenith_graph::{ImportDetail, ImportKind, ImportedNames};
use zenith_lang::ParsedFile;
use zenith_utils::location::{ByteRange, Located};

use crate::testing::Project;

/// The files of `app` in the workspace, relative to the root.
const FILES: &[&str] = &[
    "crates/app/build.rs",
    "crates/app/src/lib.rs",
    "crates/app/src/main.rs",
    "crates/app/src/a.rs",
    "crates/app/src/a/b.rs",
    "crates/app/src/c/mod.rs",
    "crates/app/src/c/d.rs",
    "crates/app/src/bin/tool.rs",
    "crates/app/tests/it.rs",
    "crates/app/tests/common/mod.rs",
    "crates/app/benches/speed.rs",
    "crates/app/examples/demo.rs",
];

/// The files of [`FILES`] whose imports are test imports once the files of
/// the workspace are marked, where each file declares the `test_modules`
/// given for it, and every file has an import and a non-literal one.
fn test_files(test_modules: &[(&str, &str)]) -> Vec<&'static str> {
    let project = Project::workspace();
    let paths: Vec<PathBuf> = FILES.iter().map(|file| project.path(file)).collect();
    let mut parsed: Vec<ParsedFile> = FILES
        .iter()
        .map(|file| {
            let declared = test_modules.iter().filter(|(declarer, _)| declarer == file);
            ParsedFile {
                imports: vec![ImportDetail {
                    range: ByteRange::new(0, 1),
                    specifier: "crate::x".into(),
                    kind: ImportKind::Value,
                    names: ImportedNames::All,
                }],
                non_literal_imports: ThinVec::from([Located::new(
                    ByteRange::new(2, 3),
                    ImportKind::Value,
                )]),
                test_modules: declared.map(|(_, module)| module.to_string()).collect(),
                ..ParsedFile::default()
            }
        })
        .collect();
    let mut files: Vec<(&Path, &mut ParsedFile)> =
        paths.iter().map(PathBuf::as_path).zip(&mut parsed).collect();
    project.crates().mark_test_code(&mut files);
    FILES
        .iter()
        .zip(&parsed)
        .filter(|(_, parsed)| {
            let test = parsed.imports[0].kind == ImportKind::Test;
            assert_eq!(parsed.non_literal_imports[0].value == ImportKind::Test, test);
            test
        })
        .map(|(file, _)| *file)
        .collect()
}

#[test]
fn the_tests_benchmarks_and_examples_are_test_code() {
    assert_eq!(
        test_files(&[]),
        [
            "crates/app/tests/it.rs",
            "crates/app/tests/common/mod.rs",
            "crates/app/benches/speed.rs",
            "crates/app/examples/demo.rs",
        ]
    );
}

#[test]
fn a_module_declared_for_tests_is_test_code_with_the_modules_in_it() {
    let declared = [("crates/app/src/lib.rs", "c"), ("crates/app/src/a.rs", "b")];
    assert_eq!(
        test_files(&declared)[..4],
        [
            "crates/app/src/a/b.rs",
            "crates/app/src/c/mod.rs",
            "crates/app/src/c/d.rs",
            "crates/app/tests/it.rs"
        ]
    );
}

#[test]
fn a_module_declared_for_tests_is_one_of_the_tree_of_the_declaring_file() {
    // The binary's `c` is not the library's, and an inline module's path goes
    // through it.
    let declared = [("crates/app/src/main.rs", "c"), ("crates/app/src/lib.rs", "a::b")];
    assert_eq!(test_files(&declared)[..1], ["crates/app/src/a/b.rs"]);
}
