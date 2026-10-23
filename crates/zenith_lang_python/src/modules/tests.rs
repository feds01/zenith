//! Tests of the roots of a project and of finding the file of a module.

use super::*;
use crate::testing::Project;

/// The module that `names` lead to from `dir`, relative to the root of
/// `project`, as its file relative to the root and whether it is regular.
fn find(project: &Project, dir: &str, names: &str) -> Option<(String, bool)> {
    let names: Vec<_> = names.split('.').collect();
    let module = Module::find(&Directories::default(), &project.path(dir), &names)?;
    let file = module.file.strip_prefix(project.root()).unwrap();
    Some((file.to_string_lossy().replace('\\', "/"), module.regular))
}

#[test]
fn a_package_is_its_init_file_and_a_module_its_file() {
    let project = Project::workspace();
    let find = |names| find(&project, "services/core", names);
    assert_eq!(find("core"), Some(("services/core/core/__init__.py".into(), true)));
    assert_eq!(find("core.db"), Some(("services/core/core/db.py".into(), true)));
    // What follows a module is names in it, and what isn't a module is a name in
    // the package.
    assert_eq!(find("core.db.Session.query"), Some(("services/core/core/db.py".into(), true)));
    assert_eq!(find("core.missing"), Some(("services/core/core/__init__.py".into(), true)));
    assert_eq!(find("missing"), None);
}

#[test]
fn a_namespace_package_leads_on_but_is_no_module() {
    let project = Project::workspace();
    let find = |names| find(&project, "services/api/src", names);
    assert_eq!(
        find("acme.api.models.user"),
        Some(("services/api/src/acme/api/models/user.py".into(), false))
    );
    assert_eq!(find("acme"), None);
    assert_eq!(find("acme.missing"), None);
}

#[test]
fn names_are_compared_exactly() {
    let project = Project::workspace();
    assert_eq!(find(&project, "services/core", "Core"), None);
    let found = find(&project, "services/core", "core.DB");
    assert_eq!(found, Some(("services/core/core/__init__.py".into(), true)));
}

#[test]
fn a_stub_is_the_file_of_a_module_without_a_source() {
    let project = Project::workspace();
    let found = find(&project, "services/api/src", "acme.api.stubs");
    assert_eq!(found, Some(("services/api/src/acme/api/stubs.pyi".into(), false)));
}

#[test]
fn the_roots_with_a_name_are_those_around_the_file_first() {
    let project = Project::new(&[
        ("a/shared/__init__.py", ""),
        ("b/shared.py", ""),
        ("b/inner/x.py", ""),
        ("c/other.py", ""),
    ]);
    let roots = Roots::configured(project.root(), &["a".into(), "b".into(), "c".into()]);
    assert_eq!(
        roots.having("shared", &project.path("c/other.py")),
        [project.path("a"), project.path("b")]
    );
    assert_eq!(
        roots.having("shared", &project.path("b/inner/x.py")),
        [project.path("b"), project.path("a")]
    );
    assert!(roots.having("missing", &project.path("c/other.py")).is_empty());
}

#[test]
fn a_configured_root_that_is_not_a_directory_is_left_out() {
    let project = Project::new(&[("src/a.py", "")]);
    let roots =
        Roots::configured(project.root(), &["src".into(), "missing".into(), "./src".into()]);
    assert_eq!(roots.dirs, [project.path("src")]);
}

#[test]
fn the_package_root_is_above_the_outermost_regular_package() {
    let project = Project::workspace();
    let directories = Directories::default();
    let root = |file| directories.package_root(&project.path(file), project.root()).to_path_buf();
    assert_eq!(root("legacy/app/views.py"), project.path("legacy"));
    assert_eq!(root("scripts/build.py"), project.path("scripts"));
    assert_eq!(
        root("services/api/src/acme/api/models/user.py"),
        project.path("services/api/src/acme")
    );
}
