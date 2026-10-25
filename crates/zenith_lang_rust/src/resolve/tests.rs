//! Tests of the resolver, on the workspace of [`Project::workspace`]: paths
//! from the crate, the module and its parents, from other crates, and from
//! the standard library.

use std::{io, path::PathBuf};

use zenith_utils::fs::{DirEntry, FileKind, OsFileSystem, disk};

use super::*;
use crate::testing::Project;

/// The workspace, its crates, and what resolving needs of them.
struct Fixture {
    project: Project,
    crates: Crates,
}

impl Fixture {
    fn new() -> Self {
        let project = Project::workspace();
        let crates = project.crates();
        Self { project, crates }
    }

    /// What `specifier` resolves to when the file `from` imports it, both
    /// relative to the root of the project.
    fn resolve(&self, from: &str, specifier: &str) -> Resolved {
        Resolver::new(self.project.root(), disk().as_ref(), &self.crates, &[])
            .resolve(&self.project.path(from), specifier)
    }

    /// The name of the file that `specifier` resolves to, which must be in the
    /// project.
    fn file(&self, from: &str, specifier: &str) -> String {
        match self.resolve(from, specifier) {
            Resolved::Internal(name) => name,
            other => panic!("{from}: {specifier} resolved to {other:?}"),
        }
    }

    /// Why `specifier` does not resolve, which it must not.
    fn failure(&self, from: &str, specifier: &str) -> String {
        match self.resolve(from, specifier) {
            Resolved::Unresolved(reason) | Resolved::Ignored(reason) => reason.to_string(),
            other => panic!("{from}: {specifier} resolved to {other:?}"),
        }
    }
}

const LIB: &str = "crates/app/src/lib.rs";
const A: &str = "crates/app/src/a.rs";
const AB: &str = "crates/app/src/a/b.rs";
const C: &str = "crates/app/src/c/mod.rs";
const CD: &str = "crates/app/src/c/d.rs";

#[test]
fn crate_paths_start_at_the_root_of_the_crate() {
    let fixture = Fixture::new();
    // From every file but the one that they are in.
    for (specifier, file) in [
        ("crate::a::b::Thing", AB),
        ("crate::a::Thing", A),
        ("crate::c::d", CD),
        ("crate::Thing", LIB),
    ] {
        for from in [LIB, A, AB, C, CD].into_iter().filter(|from| *from != file) {
            assert_eq!(fixture.file(from, specifier), file, "{from}: {specifier}");
        }
    }
}

#[test]
fn a_module_is_a_file_or_a_directory_with_mod_rs() {
    let fixture = Fixture::new();
    assert_eq!(fixture.file(LIB, "crate::a"), A);
    assert_eq!(fixture.file(LIB, "crate::c"), C);
    // Only the longest prefix that is a file counts, so what follows is an item.
    assert_eq!(fixture.file(LIB, "crate::c::d::one::two"), CD);
    assert_eq!(fixture.file(LIB, "crate::c::missing::Thing"), C);
}

#[test]
fn self_and_super_are_relative_to_the_module_of_the_file() {
    let fixture = Fixture::new();
    assert_eq!(fixture.file(A, "self::b::Thing"), AB);
    assert_eq!(fixture.file(A, "super::Thing"), LIB);
    assert_eq!(fixture.file(A, "super::c::d"), CD);
    assert_eq!(fixture.file(AB, "super::Thing"), A);
    assert_eq!(fixture.file(AB, "super::super::c"), C);
    assert_eq!(fixture.file(AB, "self::super::super::Thing"), LIB);
    assert_eq!(fixture.file(C, "self::d::Thing"), CD);
    assert_eq!(fixture.file(CD, "super::Thing"), C);
    assert_eq!(fixture.file(CD, "super::super::a::b"), AB);
    assert_eq!(fixture.file(LIB, "self::a"), A);
}

#[test]
fn a_path_into_the_importing_file_is_ignored() {
    let fixture = Fixture::new();
    for (from, specifier) in
        [(AB, "self::Thing"), (AB, "crate::a::b::Thing"), (LIB, "crate::Thing"), (A, "self")]
    {
        let resolved = fixture.resolve(from, specifier);
        assert!(matches!(resolved, Resolved::Ignored(_)), "{from}: {specifier}: {resolved:?}");
    }
}

#[test]
fn super_cannot_go_above_the_crate_root() {
    let fixture = Fixture::new();
    assert_eq!(fixture.failure(LIB, "super::a"), "`super` goes above the crate root");
    assert_eq!(fixture.failure(A, "super::super::a"), "`super` goes above the crate root");
}

#[test]
fn a_path_may_start_with_a_module_of_the_module() {
    let fixture = Fixture::new();
    assert_eq!(fixture.file(LIB, "a::b::Thing"), AB);
    assert_eq!(fixture.file(A, "b::Thing"), AB);
    assert_eq!(fixture.file(C, "d::Thing"), CD);
    // Not a module that is not there, or one of the parent.
    assert_eq!(fixture.failure(CD, "d::Thing"), "`d` is not a crate, or a module here");
    assert_eq!(fixture.failure(A, "c::d"), "`c` is not a crate, or a module here");
    // Nor when the path starts with `::`, which is for crates.
    assert_eq!(fixture.failure(A, "::b::Thing"), "`b` is not a crate, or a module here");
}

#[test]
fn a_crate_is_resolved_in_its_library() {
    let fixture = Fixture::new();
    let core = |file: &str| format!("crates/core-lib/src/{file}");
    for from in [LIB, AB] {
        assert_eq!(fixture.file(from, "core_lib::model::M"), core("model.rs"));
        assert_eq!(fixture.file(from, "::core_lib::store"), core("store/mod.rs"));
        assert_eq!(fixture.file(from, "core_lib::Thing"), core("lib.rs"));
    }
    // A crate may use its own library by its name.
    assert_eq!(fixture.file(&core("store/mod.rs"), "core_lib::model::M"), core("model.rs"));
    // The name is the dependency's, and the root of its library may be elsewhere.
    assert_eq!(fixture.file(LIB, "util::helper::run"), "crates/util/lib/helper.rs");
    assert_eq!(fixture.file(LIB, "util::Thing"), "crates/util/lib/lib.rs");
    assert_eq!(fixture.file("crates/util/lib/helper.rs", "kit::Thing"), "crates/util/lib/lib.rs");
}

#[test]
fn a_crate_that_is_not_a_dependency_is_unknown() {
    let fixture = Fixture::new();
    // `core-lib` doesn't depend on `app`, and `kit` is not `util` to `app`.
    let reason = "`app` is not a crate, or a module here";
    assert_eq!(fixture.failure("crates/core-lib/src/lib.rs", "app::a"), reason);
    let reason = "`kit` is not a crate, or a module here";
    assert_eq!(fixture.failure(LIB, "kit::helper"), reason);
}

#[test]
fn tests_benchmarks_examples_and_binaries_use_the_library_by_its_name() {
    let fixture = Fixture::new();
    for from in [
        "crates/app/tests/it.rs",
        "crates/app/tests/common/mod.rs",
        "crates/app/benches/speed.rs",
        "crates/app/examples/demo.rs",
        "crates/app/src/main.rs",
        "crates/app/src/bin/tool.rs",
        "crates/app/src/bin/multi/main.rs",
        "crates/app/build.rs",
        // Targets that the manifest gives a path.
        "crates/app/tools/extra.rs",
        "crates/app/checks/custom.rs",
    ] {
        assert_eq!(fixture.file(from, "app::a::b::Thing"), AB, "{from}");
        assert_eq!(fixture.file(from, "core_lib::model"), "crates/core-lib/src/model.rs", "{from}");
    }
}

#[test]
fn each_test_and_binary_is_a_crate_of_its_own() {
    let fixture = Fixture::new();
    let (it, common) = ("crates/app/tests/it.rs", "crates/app/tests/common/mod.rs");
    let part = "crates/app/src/bin/multi/part.rs";
    // The modules of a test or a binary are files next to it.
    assert_eq!(fixture.file(it, "crate::common::helper"), common);
    assert_eq!(fixture.file(it, "common::helper"), common);
    assert_eq!(fixture.file(part, "crate::Thing"), "crates/app/src/bin/multi/main.rs");
    assert_eq!(fixture.file(part, "super::Thing"), "crates/app/src/bin/multi/main.rs");
    // They are not the modules of the library, so `a::b` is an item of the test.
    assert_eq!(fixture.failure(it, "crate::a::b"), "the path is in the importing file");
    // A file next to a test is a module of whichever test comes first.
    assert_eq!(fixture.file(common, "super::Thing"), it);
}

#[test]
fn the_standard_library_is_builtin() {
    let fixture = Fixture::new();
    for name in ["std", "core", "alloc", "proc_macro", "test"] {
        let resolved = fixture.resolve(AB, &format!("{name}::fmt::Display"));
        assert_eq!(resolved, Resolved::Builtin(format!("rust:{name}::fmt::Display")));
    }
    assert_eq!(fixture.resolve(AB, "::std::fs"), Resolved::Builtin("rust:std::fs".into()));
}

#[test]
fn a_path_into_the_standard_library_ends_at_its_first_type() {
    let fixture = Fixture::new();
    let builtin = |path: &str| Resolved::Builtin(path.into());
    let paths = [
        ("std", "rust:std"),
        ("std::thread", "rust:std::thread"),
        ("std::thread::spawn", "rust:std::thread::spawn"),
        ("std::time::Instant", "rust:std::time::Instant"),
        ("std::collections::hash_map::Entry::Occupied", "rust:std::collections::hash_map::Entry"),
        ("std::r#f64::consts::PI", "rust:std::f64::consts::PI"),
    ];
    for (specifier, node) in paths {
        assert_eq!(fixture.resolve(AB, specifier), builtin(node), "{specifier}");
    }
}

#[test]
fn other_crates_are_external_packages() {
    let fixture = Fixture::new();
    let external = |package: &str| Resolved::External { package: package.into(), found: true };
    assert_eq!(fixture.resolve(AB, "serde::Deserialize"), external("serde"));
    // Under the name that its package has, not the one that the code gives it.
    assert_eq!(fixture.resolve(AB, "json::Value"), external("serde_json"));
    assert_eq!(fixture.resolve(AB, "serde_yaml::from_str"), external("serde-yaml"));
    // Dev, build and target dependencies are all the code's dependencies.
    assert_eq!(fixture.resolve(AB, "tempfile::tempdir"), external("tempfile"));
    assert_eq!(fixture.resolve(AB, "cc::Build"), external("cc"));
    assert_eq!(fixture.resolve(AB, "libc::c_int"), external("libc"));
}

#[test]
fn an_unknown_first_segment_does_not_resolve() {
    let fixture = Fixture::new();
    let reason = fixture.failure(AB, "missing::thing");
    assert_eq!(reason, "`missing` is not a crate, or a module here");
}

#[test]
fn a_raw_identifier_names_what_it_spells() {
    let fixture = Fixture::new();
    assert_eq!(fixture.file(LIB, "crate::r#a::r#b::Thing"), AB);
}

#[test]
fn paths_that_match_ignore_unresolved_are_ignored() {
    let fixture = Fixture::new();
    let patterns = [NamePattern::new("generated::*").unwrap()];
    let resolver =
        Resolver::new(fixture.project.root(), disk().as_ref(), &fixture.crates, &patterns);
    let resolve = |specifier| resolver.resolve(&fixture.project.path(AB), specifier);
    assert!(matches!(resolve("generated::Thing"), Resolved::Ignored(_)));
    assert!(matches!(resolve("missing::Thing"), Resolved::Unresolved(_)));
}

/// The disk, with names that ignore case, as macOS's and Windows' do by
/// default, wherever the tests run.
#[derive(Debug)]
struct IgnoringCase;

impl IgnoringCase {
    /// `path`, with each component spelt as its directory lists it.
    fn spelt(path: &Path) -> PathBuf {
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return path.to_path_buf();
        };
        let dir = Self::spelt(dir);
        let entries = OsFileSystem.read_dir(&dir).unwrap_or_default();
        (entries.into_iter())
            .find(|entry| entry.path.file_name().is_some_and(|it| it.eq_ignore_ascii_case(name)))
            .map_or_else(|| dir.join(name), |entry| entry.path)
    }
}

impl FileSystem for IgnoringCase {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        OsFileSystem.read(&Self::spelt(path))
    }

    fn kind(&self, path: &Path) -> io::Result<FileKind> {
        OsFileSystem.kind(&Self::spelt(path))
    }

    fn link_kind(&self, path: &Path) -> io::Result<FileKind> {
        OsFileSystem.link_kind(&Self::spelt(path))
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        OsFileSystem.read_dir(&Self::spelt(path))
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        OsFileSystem.read_link(&Self::spelt(path))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        OsFileSystem.canonicalize(&Self::spelt(path))
    }
}

#[test]
fn a_module_is_spelt_as_its_file_where_names_ignore_case() {
    let fixture = Fixture::new();
    let resolver = Resolver::new(fixture.project.root(), &IgnoringCase, &fixture.crates, &[]);
    let file = |from, specifier| match resolver.resolve(&fixture.project.path(from), specifier) {
        Resolved::Internal(name) => name,
        other => panic!("{from}: {specifier} resolved to {other:?}"),
    };
    // A type that the root re-exports, not `a.rs` or `c/mod.rs`.
    assert_eq!(file(AB, "crate::A"), LIB);
    assert_eq!(file(AB, "crate::C::d"), LIB);
    assert_eq!(file(CD, "crate::a::B"), A);
    assert_eq!(file(LIB, "core_lib::Model"), "crates/core-lib/src/lib.rs");
    // The modules themselves still are.
    assert_eq!(file(LIB, "crate::a::b"), AB);
    assert_eq!(file(LIB, "crate::c::d"), CD);
}

#[test]
fn a_tool_attribute_or_lint_is_ignored() {
    let fixture = Fixture::new();
    for specifier in ["clippy::too_many_lines", "rustfmt::skip", "diagnostic::on_unimplemented"] {
        let resolved = fixture.resolve(AB, specifier);
        assert!(matches!(resolved, Resolved::Ignored(_)), "{specifier}: {resolved:?}");
    }
    // A crate or module of the tool's name is what the path names.
    assert_eq!(fixture.failure(AB, "::clippy::Lint"), "`clippy` is not a crate, or a module here");
}

#[test]
fn a_type_in_scope_is_ignored() {
    let fixture = Fixture::new();
    let resolved = fixture.resolve(AB, "Direction::North");
    assert!(matches!(resolved, Resolved::Ignored(_)), "{resolved:?}");
}

#[test]
fn a_file_outside_every_crate_has_no_crate_paths() {
    let fixture = Fixture::new();
    let reason = fixture.failure("scripts/run.rs", "crate::a");
    assert_eq!(reason, "the file is in no Cargo package");
}

#[test]
fn a_file_outside_every_module_tree_has_only_crates_to_import() {
    let fixture = Fixture::new();
    let stray = "crates/app/notes/stray.rs";
    assert_eq!(fixture.failure(stray, "crate::a"), "the file is in no module tree of `app`");
    assert_eq!(fixture.resolve(stray, "std::fs"), Resolved::Builtin("rust:std::fs".into()));
    assert_eq!(fixture.file(stray, "app::a"), A);
}

#[test]
fn a_crate_without_a_library_does_not_resolve() {
    let project = Project::new(&[
        ("bin/Cargo.toml", include_str!("../fixtures/bin-only.toml")),
        ("bin/src/main.rs", ""),
        ("user/Cargo.toml", include_str!("../fixtures/uses-bin-only.toml")),
        ("user/src/lib.rs", ""),
    ]);
    let crates = project.crates();
    let resolved = Resolver::new(project.root(), disk().as_ref(), &crates, &[])
        .resolve(&project.path("user/src/lib.rs"), "bin_only::a");
    assert_eq!(resolved, Resolved::failed("`bin-only` has no library"));
}
