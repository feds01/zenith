//! Tests of contracts on imports of packages: directories that are imported
//! as a whole, as in Go, so that an import names a directory, not a file.
//!
//! Each test checks contracts of `fixtures/packages.toml` against a project
//! whose imports of a name that ends in `/` are imports of the package in that
//! directory.

use zenith_graph::{ImportKind, Scope};

use crate::{
    Violation,
    testing::{Project, values},
};

/// The imports of the project, on lines numbered from 1 in this order.
///
/// `cmd` imports the packages `internal/db` and `internal/log`. The package
/// `internal/db` has two files, which import the packages `internal/cache` and
/// `internal/log`, and a subdirectory, whose file imports `internal/secret`.
/// `internal/cache` imports `internal/store`, so that a chain goes through
/// three packages. `cyc/a` and `cyc/b` import each other's packages.
const IMPORTS: [(&str, &str); 13] = [
    ("cmd/app/main.go", "internal/db/"),
    ("cmd/app/main.go", "internal/log/"),
    ("internal/db/conn.go", "internal/cache/"),
    ("internal/db/query.go", "internal/log/"),
    ("internal/cache/store.go", "internal/log/"),
    ("internal/log/log.go", "node:os"),
    ("internal/db/sub/deep.go", "internal/secret/"),
    ("internal/secret/key.go", "node:os"),
    ("cyc/a/x.go", "cyc/b/"),
    ("cyc/b/y.go", "cyc/a/"),
    ("tests/db_test.go", "internal/db/"),
    ("internal/cache/store.go", "internal/store/"),
    ("internal/store/disk.go", "node:os"),
];

/// A project of `imports`, with the contracts of the fixture.
fn project(imports: &[(&str, &str)]) -> Project {
    Project::new(&values(imports), &[], include_str!("fixtures/packages.toml"))
}

/// The project of [`IMPORTS`].
fn standard() -> Project {
    project(&IMPORTS)
}

/// The project of [`IMPORTS`] and `more`.
fn with(more: &[(&str, &str)]) -> Project {
    project(&[&IMPORTS[..], more].concat())
}

#[test]
fn an_import_of_a_package_is_one_finding_named_by_its_directory() {
    let project = standard();
    // Two files make up the package, but one import is one finding.
    assert_eq!(
        project.findings("of-a-package"),
        ["cmd -> internal/db: cmd/app/main.go -> internal/db"]
    );
    assert_eq!(project.check("of-a-package").finding_count(), 1);
}

#[test]
fn a_package_imported_twice_is_one_finding_with_both_statements() {
    let project = with(&[("cmd/app/main.go", "internal/db/")]);
    let report = project.check("of-a-package");
    assert_eq!(report.finding_count(), 1);
    let Violation::Dependency { chains, .. } = &report.violations[0] else {
        panic!("expected a dependency");
    };
    assert_eq!(chains[0].links.len(), 1);
    assert_eq!(chains[0].links[0].imports.len(), 2);
}

#[test]
fn a_chain_goes_through_a_package_by_the_imports_that_make_it() {
    let project = standard();
    assert_eq!(
        project.findings("through-packages"),
        ["cmd -> internal/cache: cmd/app/main.go -> internal/db -> internal/db/conn.go \
             -> internal/cache"]
    );

    // Each link of the chain is an import: the first is made by `cmd`, on line 1,
    // and the second by a file of the package, on line 3. No link stands for
    // the package and its file.
    let report = project.check("through-packages");
    let Violation::Dependency { chains, .. } = &report.violations[0] else {
        panic!("expected a dependency");
    };
    let hops: Vec<_> = chains[0]
        .links
        .iter()
        .map(|link| {
            let starts: Vec<_> = link.imports.iter().map(|import| import.range.start()).collect();
            (project.graph.name(link.importer), project.graph.name(link.imported), starts)
        })
        .collect();
    assert_eq!(
        hops,
        [
            ("cmd/app/main.go", "internal/db", vec![100]),
            ("internal/db/conn.go", "internal/cache", vec![300]),
        ]
    );
}

#[test]
fn a_chain_goes_through_a_chain_of_three_packages() {
    let project = standard();
    assert_eq!(
        project.findings("through-three-packages"),
        ["cmd -> internal/store: cmd/app/main.go -> internal/db -> internal/db/conn.go \
             -> internal/cache -> internal/cache/store.go -> internal/store"]
    );
    let report = project.check("through-three-packages");
    assert_eq!(report.finding_count(), 1);
    let Violation::Dependency { chains, .. } = &report.violations[0] else {
        panic!("expected a dependency");
    };
    let imports: Vec<_> = chains[0]
        .links
        .iter()
        .map(|link| (project.graph.name(link.importer), project.graph.name(link.imported)))
        .collect();
    assert_eq!(
        imports,
        [
            ("cmd/app/main.go", "internal/db"),
            ("internal/db/conn.go", "internal/cache"),
            ("internal/cache/store.go", "internal/store"),
        ]
    );
}

#[test]
fn a_package_does_not_reach_what_its_subdirectories_import() {
    assert!(standard().check("not-through-a-subdirectory").is_kept());
}

#[test]
fn only_direct_imports_of_packages_count_when_indirect_ones_are_allowed() {
    let project = standard();
    assert!(project.check("direct-only").is_kept());
    assert_eq!(
        with(&[("cmd/app/other.go", "internal/cache/")]).findings("direct-only"),
        ["cmd -> internal/cache: cmd/app/other.go -> internal/cache"]
    );
}

#[test]
fn nothing_between_a_package_and_its_files_is_an_import_of_a_kind() {
    // Excluding every kind but values leaves the chain, so no kind stands for
    // the hop from the package to its file.
    let project = standard();
    assert_eq!(project.findings("every-kind-but-values"), project.findings("through-packages"));

    // An import of a package is of a kind, like any other.
    let imports = [
        ("cmd/app/main.go", "internal/db/", ImportKind::Type),
        ("internal/db/conn.go", "internal/cache/", ImportKind::Value),
    ];
    let project = Project::new(&imports, &[], include_str!("fixtures/packages.toml"));
    assert!(project.check("every-kind-but-values").is_kept());
    assert_eq!(project.findings("through-packages").len(), 1);
}

#[test]
fn an_import_of_a_package_is_counted_once_as_an_import() {
    let project = standard();
    assert_eq!(project.graph.detail_count(), IMPORTS.len());
    assert_eq!(project.graph.edges().count(), IMPORTS.len());
}

#[test]
fn ignored_imports_match_packages() {
    let project = standard();
    let report = project.check("ignored-package");
    assert!(report.is_kept());
    assert!(report.unmatched_ignores.is_empty());
    assert_eq!(report.ignored, 1);

    // An import in the middle of a chain can be ignored too.
    let report = project.check("ignored-inside-the-chain");
    assert!(report.is_kept());
    assert!(report.unmatched_ignores.is_empty());
    assert_eq!(report.ignored, 1);
}

#[test]
fn ignored_imports_that_match_no_import_are_unmatched() {
    let project = standard();
    let report = project.check("ignored-nothing");
    let unmatched: Vec<_> =
        report.unmatched_ignores.iter().map(|entry| entry.to_string()).collect();
    assert_eq!(unmatched, ["cmd/** -> internal/cache", "cmd/** -> internal/secret"]);
    assert_eq!(project.findings("ignored-nothing").len(), 1);
}

#[test]
fn layers_see_the_imports_of_packages() {
    let project = standard();
    assert_eq!(
        project.findings("layers-direct"),
        ["internal/db -> internal/cache: internal/db/conn.go -> internal/cache"]
    );
    assert_eq!(
        project.findings("layers-through"),
        ["cmd -> internal/cache: cmd/app/main.go -> internal/db -> internal/db/conn.go \
             -> internal/cache"]
    );
}

#[test]
fn independent_modules_see_the_imports_of_packages_and_the_chains_through_them() {
    assert_eq!(
        standard().findings("independent"),
        [
            "internal/db -> internal/log: internal/db/query.go -> internal/log",
            "internal/db -> internal/log: internal/db/conn.go -> internal/cache -> \
             internal/cache/store.go -> internal/log",
        ]
    );
}

#[test]
fn a_protected_package_may_only_be_imported_by_allowed_importers() {
    // `tests` is allowed, and the files in the package are its own: the package
    // is not an importer of them.
    assert_eq!(standard().findings("protected"), ["internal/db: cmd/app/main.go -> internal/db"]);
    assert!(
        project(&[("tests/db_test.go", "internal/db/"), ("internal/db/conn.go", "node:os")])
            .check("protected")
            .is_kept()
    );
}

#[test]
fn an_encapsulated_module_may_only_be_imported_through_entry_point_packages() {
    // `internal/log` is the entry point. The other packages of the module are
    // imported from outside it, and the package is not an importer of its
    // own files.
    assert_eq!(
        standard().findings("encapsulated"),
        ["internal: cmd/app/main.go -> internal/db", "internal: tests/db_test.go -> internal/db",]
    );
}

#[test]
fn packages_that_import_each_other_are_a_cycle_but_a_package_and_its_files_are_not() {
    let project = standard();
    assert_eq!(
        project.findings("cycles"),
        ["cyc/*: break cyc/b -> cyc/a (cyc/b/y.go -> cyc/a) in cyc/b -> cyc/a"]
    );
    assert!(project.check("no-cycles").is_kept());
}

#[test]
fn baseline_entries_name_the_package_so_they_survive_a_new_file_in_it() {
    let project = standard();
    assert_eq!(project.baseline_entries("of-a-package"), ["cmd/app/main.go -> internal/db"]);
    assert_eq!(project.baseline_entries("through-packages"), ["cmd/app/main.go -> internal/db"]);

    // Recorded in the baseline, the entry keeps the contract, in a project
    // with a new file in the package, whose imports are not recorded.
    let entries = ["cmd/app/main.go -> internal/db"];
    for id in ["of-a-package", "through-packages"] {
        let grown = with(&[
            ("internal/db/extra.go", "internal/store/"),
            ("internal/db/more.go", "node:os"),
        ])
        .baseline(id, &entries);
        let report = grown.check(id);
        assert!(report.is_kept(), "{id}");
        assert_eq!((report.baselined, report.stale_entries.len()), (1, 0), "{id}");
    }

    // Another importer of the package is a new violation, not covered by the entry.
    let grown = with(&[("cmd/app/new.go", "internal/db/")]).baseline("through-packages", &entries);
    assert_eq!(
        grown.findings("through-packages"),
        ["cmd -> internal/cache: cmd/app/new.go -> internal/db -> internal/db/conn.go \
             -> internal/cache"]
    );
}

#[test]
fn a_baseline_entry_of_a_removed_package_import_is_stale() {
    let project = project(&[
        ("cmd/app/main.go", "internal/cache/"),
        ("internal/db/conn.go", "internal/cache/"),
    ])
    .baseline("of-a-package", &["cmd/app/main.go -> internal/db"]);
    let report = project.check("of-a-package");
    assert_eq!(report.stale_entries, ["cmd/app/main.go -> internal/db"]);
}

#[test]
fn suppressing_the_statement_of_a_package_import_leaves_it_out() {
    let project = standard().suppress("cmd/app/main.go", 1, "of-a-package through-packages");
    for id in ["of-a-package", "through-packages"] {
        let report = project.check(id);
        assert!(report.is_kept(), "{id}");
        assert_eq!(report.suppressed, 1, "{id}");
    }

    // The import in the middle of a chain.
    let project = standard().suppress("internal/db/conn.go", 3, "through-packages");
    assert!(project.check("through-packages").is_kept());
}

#[test]
fn an_import_of_a_package_is_in_the_scope_of_its_files() {
    let project = standard();
    let findings = |path: &str| {
        let mut report = project.check("of-a-package");
        report.narrow(&project.graph, &Scope::new(vec![path.to_string()]));
        report.finding_count()
    };
    // The importer, the package, and a file directly in it, but not what is
    // in a subdirectory of the package.
    assert_eq!(findings("cmd/app/main.go"), 1);
    assert_eq!(findings("internal/db"), 1);
    assert_eq!(findings("internal/db/query.go"), 1);
    assert_eq!(findings("internal/db/sub/deep.go"), 0);
    assert_eq!(findings("internal/log"), 0);
}
