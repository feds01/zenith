//! Tests of discovery: the crates of a project, their names, and what they
//! depend on.

use zenith_lang::WorkspacePackage;

use super::*;
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
    let project = Project::new(&[
        ("Cargo.toml", include_str!("../fixtures/root-package.toml")),
        ("member/Cargo.toml", include_str!("../fixtures/member.toml")),
    ]);
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
fn a_crate_is_named_for_its_package_unless_its_library_says_otherwise() {
    let crates = Project::workspace().crates();
    let (lib, util) = (krate(&crates, "core-lib"), krate(&crates, "util-kit"));
    assert_eq!((&lib.lib_name[..], &lib.lib_path[..]), ("core_lib", "src/lib.rs"));
    assert_eq!((&util.lib_name[..], &util.lib_path[..]), ("kit", "lib/lib.rs"));
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
            // A path to a crate of the project, whose library is called `kit`.
            ("util", &internal("util-kit")),
        ]
    );
    let util = krate(&crates, "util-kit");
    assert_eq!(dependencies(util), [("kit", &internal("util-kit"))]);
}

#[test]
fn workspace_dependencies_are_relative_to_the_workspace_root() {
    let project = Project::new(&[
        ("Cargo.toml", include_str!("../fixtures/root-package.toml")),
        ("member/Cargo.toml", include_str!("../fixtures/member.toml")),
    ]);
    let crates = project.crates();
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
