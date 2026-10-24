//! Tests of discovery: the packages of a project and the directories their
//! imports start from.

use zenith_utils::fs::disk;

use super::*;
use crate::testing::Project;

#[test]
fn every_manifest_that_names_a_package_is_a_package() {
    let project = Project::workspace();
    let workspace = Package::workspace(project.root(), &project.packages());
    let packages: Vec<_> =
        workspace.packages.iter().map(|package| (&package.name[..], &package.dir[..])).collect();
    // The standard `[project]` names one, and Poetry's own table the other.
    assert_eq!(packages, [("acme-api", "services/api"), ("acme-core", "services/core")]);
    // The root only configures tools.
    assert_eq!(workspace.root_package, None);
    assert_eq!(workspace.packages[0].path, project.path("services/api"));
}

#[test]
fn a_package_in_the_root_is_the_root_package() {
    let project = Project::new(&[("pyproject.toml", include_str!("../fixtures/api.toml"))]);
    let workspace = Package::workspace(project.root(), &project.packages());
    assert!(workspace.packages.is_empty());
    assert_eq!(workspace.root_package.map(|package| package.name).as_deref(), Some("acme-api"));
}

#[test]
fn a_manifest_that_cannot_be_read_is_left_out() {
    let project = Project::new(&[
        ("a/pyproject.toml", include_str!("../fixtures/invalid.toml")),
        ("b/pyproject.toml", include_str!("../fixtures/core.toml")),
    ]);
    let names: Vec<_> = project.packages().into_iter().map(|package| package.name).collect();
    assert_eq!(names, ["acme-core"]);
}

#[test]
fn imports_start_from_src_in_the_src_layout_and_from_the_package_otherwise() {
    let project = Project::workspace();
    let roots: Vec<_> =
        project.packages().iter().map(|package| package.root(disk().as_ref())).collect();
    assert_eq!(roots, [project.path("services/api/src"), project.path("services/core")]);
}
