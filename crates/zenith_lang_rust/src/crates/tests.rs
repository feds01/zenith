//! Tests of discovery: the crates of a project, their targets, and what they
//! depend on, whether Cargo or the manifests say so.

use zenith_lang::WorkspacePackage;

use super::{
    cargo::{CargoError, asked},
    *,
};
use crate::testing::Project;

/// The crate named `name` of `crates`.
fn krate<'c>(crates: &'c Crates, name: &str) -> &'c Crate {
    crates.crates.iter().find(|krate| krate.name == name).unwrap()
}

/// The index of the crate named `name` of `crates`.
fn index(crates: &Crates, name: &str) -> usize {
    crates.crates.iter().position(|krate| krate.name == name).unwrap()
}

/// What `krate` depends on, by name, sorted.
fn dependencies(krate: &Crate) -> Vec<(&str, &Dependency)> {
    let mut dependencies: Vec<_> =
        krate.dependencies.iter().map(|(name, dependency)| (name.as_str(), dependency)).collect();
    dependencies.sort_by_key(|(name, _)| *name);
    dependencies
}

/// The crates of `project` as Cargo says they are, or `None` if it isn't
/// installed, which is said.
fn from_cargo(project: &Project) -> Option<Crates> {
    let manifests = Manifest::read_all(project.manifests());
    match Crates::from_cargo(project.root(), &manifests) {
        Ok(crates) => Some(crates),
        Err(CargoError::NotFound) => {
            eprintln!("skipping the comparison with Cargo: `cargo` is not installed");
            None
        }
        Err(error) => panic!("`cargo metadata` failed: {error}"),
    }
}

#[test]
fn every_manifest_with_a_package_is_a_crate() {
    let project = Project::workspace();
    let workspace = project.crates().workspace(project.root());
    let packages: Vec<_> =
        workspace.packages.iter().map(|package| (&package.name[..], &package.dir[..])).collect();
    assert_eq!(
        packages,
        [("app", "crates/app"), ("core-lib", "crates/core-lib"), ("util-kit", "crates/util")]
    );
    // The root only declares a workspace.
    assert_eq!(workspace.root_package, None);
    assert_eq!(workspace.packages[0].path, project.path("crates/app"));
}

#[test]
fn a_package_in_the_root_is_the_root_package() {
    let project = Project::root_package();
    let workspace = project.crates().workspace(project.root());
    let root = WorkspacePackage {
        name: "root-app".into(),
        dir: String::new(),
        path: project.root().to_path_buf(),
    };
    assert_eq!(workspace.root_package, Some(root));
    let packages: Vec<_> = workspace.packages.iter().map(|package| &package.dir[..]).collect();
    assert_eq!(packages, ["member"]);
}

#[test]
fn a_manifest_that_cannot_be_read_is_skipped() {
    let project = Project::new(&[
        ("a/Cargo.toml", include_str!("../fixtures/invalid.toml")),
        ("b/Cargo.toml", include_str!("../fixtures/core-lib.toml")),
    ]);
    let workspace = project.crates().workspace(project.root());
    let names: Vec<_> = workspace.packages.iter().map(|package| &package.name[..]).collect();
    assert_eq!(names, ["core-lib"]);
}

#[test]
fn a_crate_has_the_targets_that_cargo_finds_and_those_that_the_manifest_gives() {
    let project = Project::workspace();
    let crates = project.crates();
    let targets = |name: &str| -> Vec<(TargetKind, String)> {
        let dir = &krate(&crates, name).dir;
        (krate(&crates, name).targets.iter())
            .map(|target| {
                let root = target.root.strip_prefix(dir).unwrap();
                (target.kind, root.to_string_lossy().replace('\\', "/"))
            })
            .collect()
    };
    let target = |kind, root: &str| (kind, root.to_string());
    assert_eq!(
        targets("app"),
        [
            target(TargetKind::Lib, "src/lib.rs"),
            // `src/bin/*.rs` and `src/bin/*/main.rs`, `src/main.rs`, and `[[bin]]`.
            target(TargetKind::Bin, "src/bin/multi/main.rs"),
            target(TargetKind::Bin, "src/bin/tool.rs"),
            target(TargetKind::Bin, "src/main.rs"),
            target(TargetKind::Bin, "tools/extra.rs"),
            // `tests/*.rs`, and `[[test]]`.
            target(TargetKind::Test, "checks/custom.rs"),
            target(TargetKind::Test, "tests/it.rs"),
            target(TargetKind::Bench, "benches/speed.rs"),
            target(TargetKind::Example, "examples/demo.rs"),
            target(TargetKind::Build, "build.rs"),
        ]
    );
    // The library may be anywhere.
    assert_eq!(targets("util-kit"), [target(TargetKind::Lib, "lib/lib.rs")]);
}

#[test]
fn dependencies_are_named_as_the_code_uses_them() {
    let crates = Project::workspace().crates();
    let external = |package: &str| Dependency::External(package.to_string());
    let internal = |name: &str| Dependency::Internal(index(&crates, name));
    assert_eq!(
        dependencies(krate(&crates, "app")),
        [
            // The crate's own library, for its tests and examples.
            ("app", &internal("app")),
            // The build dependencies.
            ("cc", &external("cc")),
            // `workspace = true` takes the path from the workspace, relative to it.
            ("core_lib", &internal("core-lib")),
            // A rename is the name of the code, and the package is what is external.
            ("json", &external("serde_json")),
            // The dependencies of a target.
            ("libc", &external("libc")),
            ("serde", &external("serde")),
            // `-` is `_` in code.
            ("serde_yaml", &external("serde-yaml")),
            // The dev-dependencies.
            ("tempfile", &external("tempfile")),
            // A path to a crate of the project, which the code may name as it likes.
            ("util", &internal("util-kit")),
        ]
    );
    // The library of `util-kit` is `kit` for its own targets.
    let util = krate(&crates, "util-kit");
    assert_eq!(dependencies(util), [("kit", &internal("util-kit"))]);
}

#[test]
fn workspace_dependencies_are_relative_to_the_workspace_root() {
    let crates = Project::root_package().crates();
    let root = krate(&crates, "root-app");
    assert_eq!(root.dependencies["member"], Dependency::Internal(index(&crates, "member")));
    assert_eq!(root.dependencies["root_app"], Dependency::Internal(index(&crates, "root-app")));
    let member = krate(&crates, "member");
    assert_eq!(member.dependencies["root_app"], Dependency::Internal(index(&crates, "root-app")));
}

#[test]
fn a_file_is_in_the_nearest_crate_above_it() {
    let project = Project::workspace();
    let crates = project.crates();
    let name = |path: &str| crates.containing(&project.path(path)).map(|krate| krate.name.as_str());
    assert_eq!(name("crates/app/src/a/b.rs"), Some("app"));
    assert_eq!(name("crates/app/tests/it.rs"), Some("app"));
    assert_eq!(name("crates/util/lib/lib.rs"), Some("util-kit"));
    assert_eq!(name("crates/other/lib.rs"), None);
    assert_eq!(name("build.rs"), None);
}

#[test]
fn cargo_and_the_manifests_find_the_same_crates() {
    // A virtual workspace, renames, `workspace = true`, and every kind of target in
    // one; and a workspace with a package in its root.
    for project in [Project::workspace(), Project::root_package()] {
        if let Some(from_cargo) = from_cargo(&project) {
            assert_eq!(from_cargo, project.crates());
        }
    }
}

#[test]
fn a_workspace_is_asked_about_once() {
    let ask = |project: &Project| -> Vec<String> {
        let manifests = Manifest::read_all(project.manifests());
        let asked = asked(&manifests);
        asked
            .map(|dir| dir.strip_prefix(project.root()).unwrap().to_string_lossy().into())
            .collect()
    };
    // The members of a workspace are covered by its manifest, which is always
    // asked.
    assert_eq!(ask(&Project::workspace()), [""]);
    assert_eq!(ask(&Project::root_package()), [""]);
    // Packages that no workspace covers are asked about one by one.
    let separate = Project::new(&[
        ("a/Cargo.toml", include_str!("../fixtures/core-lib.toml")),
        ("b/Cargo.toml", include_str!("../fixtures/core-lib.toml")),
    ]);
    assert_eq!(ask(&separate), ["a", "b"]);
}

#[test]
fn the_manifests_are_read_when_cargo_fails() {
    // Cargo finds no target in a package without a file, and says so.
    let project = Project::new(&[("Cargo.toml", include_str!("../fixtures/core-lib.toml"))]);
    let found = Crates::discover(project.root(), project.manifests(), Discovery::Cargo);
    assert_eq!(found, project.crates());
    assert_eq!(found.crates.len(), 1);
}
