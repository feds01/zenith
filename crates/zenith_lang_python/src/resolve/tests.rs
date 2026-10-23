//! Tests of the resolver, on the workspace of [`Project::workspace`]:
//! absolute imports from the roots, relative imports, the standard library
//! and third-party packages.

use zenith_utils::glob::NamePattern;

use super::*;
use crate::testing::Project;

/// The workspace, its roots, and what resolving needs of them.
struct Fixture {
    project: Project,
    roots: Roots,
}

impl Fixture {
    fn new() -> Self {
        let project = Project::workspace();
        let roots = project.roots();
        Self { project, roots }
    }

    /// What `specifier` resolves to when the file `from` imports it, both
    /// relative to the root of the project.
    fn resolve(&self, from: &str, specifier: &str) -> Resolved {
        Resolver::new(self.project.root(), &self.roots, &[])
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

const HANDLERS: &str = "services/api/src/acme/api/handlers.py";
const API: &str = "services/api/src/acme/api/__init__.py";
const MODELS: &str = "services/api/src/acme/api/models/__init__.py";
const USER: &str = "services/api/src/acme/api/models/user.py";
const CORE: &str = "services/core/core/__init__.py";
const DB: &str = "services/core/core/db.py";

#[test]
fn absolute_imports_start_at_the_roots_of_the_packages() {
    let fixture = Fixture::new();
    for from in [HANDLERS, CORE, "scripts/build.py", "services/api/tests/test_handlers.py"] {
        assert_eq!(fixture.file(from, "acme.api.models.user"), USER, "{from}");
        assert_eq!(fixture.file(from, "core.db"), DB, "{from}");
    }
    assert_eq!(fixture.file(HANDLERS, "core"), CORE);
    assert_eq!(fixture.file(USER, "acme.api"), API);
}

#[test]
fn a_name_that_is_not_a_module_is_in_the_module_before_it() {
    let fixture = Fixture::new();
    // `from acme.api.models import User` and `from core.db import Session`.
    assert_eq!(fixture.file(HANDLERS, "acme.api.models.User"), MODELS);
    assert_eq!(fixture.file(HANDLERS, "core.db.Session"), DB);
}

#[test]
fn a_file_does_not_import_itself() {
    let fixture = Fixture::new();
    assert_eq!(fixture.failure(API, "acme.api.thing"), "the name is in the importing file");
    assert_eq!(fixture.failure(MODELS, ".User"), "the name is in the importing file");
}

#[test]
fn relative_imports_start_at_the_package_of_the_file() {
    let fixture = Fixture::new();
    // `from . import handlers`, `from .models import user` and `from .. import x`.
    assert_eq!(fixture.file(USER, "..handlers"), HANDLERS);
    assert_eq!(fixture.file(HANDLERS, ".models.user"), USER);
    assert_eq!(fixture.file(HANDLERS, ".models"), MODELS);
    assert_eq!(fixture.file(USER, ".User"), MODELS);
    assert_eq!(fixture.file(MODELS, ".user"), USER);
    assert_eq!(fixture.file(HANDLERS, ".Thing"), API);
    // `from . import *` in a package's `__init__.py` is the package itself.
    assert_eq!(fixture.file(USER, "."), MODELS);
}

#[test]
fn a_relative_import_of_a_namespace_package_is_its_directory() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.resolve("services/api/src/acme/api/__init__.py", ".."),
        Resolved::Package("services/api/src/acme".into())
    );
    assert_eq!(fixture.file("scripts/build.py", ".helpers"), "scripts/helpers.py");
    // A namespace package has no file for names to be in.
    assert_eq!(
        fixture.failure("services/api/src/acme/api/__init__.py", "..Thing"),
        "no module `Thing` in the namespace package"
    );
    assert_eq!(
        fixture.failure("scripts/build.py", ".missing.run"),
        "no module `missing` in the namespace package"
    );
}

#[test]
fn a_relative_import_does_not_go_above_the_project_root() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.failure("scripts/build.py", "...x"),
        "the relative import goes above the project root"
    );
}

#[test]
fn loose_files_import_each_other_from_their_own_directory() {
    let fixture = Fixture::new();
    // As when `scripts/build.py` is run, or pytest imports `tests/conftest.py`.
    assert_eq!(fixture.file("scripts/build.py", "helpers"), "scripts/helpers.py");
    assert_eq!(fixture.file("tests/conftest.py", "util.fixture"), "tests/util.py");
    // And the files of a regular package from above it, as `manage.py` would.
    assert_eq!(fixture.file("legacy/app/views.py", "app.models"), "legacy/app/models.py");
}

#[test]
fn the_standard_library_is_a_builtin_named_by_its_top_level_module() {
    let fixture = Fixture::new();
    assert_eq!(fixture.resolve(HANDLERS, "os.path"), Resolved::Builtin("python:os".into()));
    assert_eq!(fixture.resolve(HANDLERS, "typing.Any"), Resolved::Builtin("python:typing".into()));
}

#[test]
fn a_regular_module_hides_the_standard_library_but_a_namespace_package_does_not() {
    let fixture = Fixture::new();
    // `services/core/core/logging.py` is only `logging` from inside `core`.
    assert_eq!(fixture.resolve(DB, "logging"), Resolved::Builtin("python:logging".into()));
    assert_eq!(fixture.file(DB, "core.logging"), "services/core/core/logging.py");
    // `email` is a directory at the root without an `__init__.py`.
    assert_eq!(fixture.resolve(HANDLERS, "email.mime"), Resolved::Builtin("python:email".into()));
    assert_eq!(
        fixture.resolve(HANDLERS, "email.templates"),
        Resolved::Builtin("python:email".into())
    );
}

#[test]
fn a_module_in_a_namespace_package_of_the_project_is_internal() {
    let fixture = Fixture::new();
    assert_eq!(fixture.file(HANDLERS, "scripts.helpers"), "scripts/helpers.py");
    // A namespace package alone is not a module of the project.
    assert_eq!(
        fixture.resolve(HANDLERS, "scripts"),
        Resolved::External { package: "scripts".into(), found: true }
    );
}

#[test]
fn anything_else_is_a_third_party_package_named_by_its_top_level_module() {
    let fixture = Fixture::new();
    for (specifier, package) in
        [("requests", "requests"), ("requests.adapters.HTTPAdapter", "requests"), ("yaml", "yaml")]
    {
        assert_eq!(
            fixture.resolve(HANDLERS, specifier),
            Resolved::External { package: package.into(), found: true },
            "{specifier}"
        );
    }
}

#[test]
fn configured_roots_replace_those_of_the_packages() {
    let fixture = Fixture::new();
    let roots = Roots::configured(fixture.project.root(), &["services/core".into()]);
    let resolver = Resolver::new(fixture.project.root(), &roots, &[]);
    let resolve = |specifier| resolver.resolve(&fixture.project.path(CORE), specifier);
    assert_eq!(resolve("core.db"), Resolved::Internal(DB.into()));
    assert_eq!(resolve("acme.api"), Resolved::External { package: "acme".into(), found: true });
}

#[test]
fn unresolved_paths_that_match_a_pattern_are_expected() {
    let fixture = Fixture::new();
    let patterns = [NamePattern::new(".generated.*").unwrap()];
    let resolver = Resolver::new(fixture.project.root(), &fixture.roots, &patterns);
    let resolve =
        |specifier| resolver.resolve(&fixture.project.path("scripts/build.py"), specifier);
    assert_eq!(
        resolve(".generated.models"),
        Resolved::ignored("no module `generated` in the namespace package")
    );
    assert_eq!(
        resolve(".missing.run"),
        Resolved::failed("no module `missing` in the namespace package")
    );
}
