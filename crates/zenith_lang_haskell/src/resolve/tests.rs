//! Tests of the resolver, on the project of [`Project::acme`]: the modules
//! of a component, of the libraries that it depends on, of the project's and
//! the build's other libraries, and the imports that name packages.

use zenith_utils::{fs::disk, glob::NamePattern};

use super::*;
use crate::testing::Project;

/// The acme project, and its packages, built or not.
struct Fixture {
    project: Project,
    packages: Packages,
}

impl Fixture {
    /// The project, as built.
    fn built() -> Self {
        let project = Project::acme();
        let packages = project.packages();
        Self { project, packages }
    }

    /// The project, never built.
    fn unbuilt() -> Self {
        let project = Project::acme();
        let packages = project.unbuilt();
        Self { project, packages }
    }

    /// What `specifier` resolves to when the file `from` imports it, relative
    /// to the root of the project, which expects the specifiers that match
    /// `ignore_unresolved` not to resolve.
    fn resolve_ignoring(
        &self,
        from: &str,
        specifier: &str,
        ignore_unresolved: &[NamePattern],
    ) -> Resolved {
        Resolver::new(self.project.root(), &**disk(), &self.packages, ignore_unresolved)
            .resolve(&self.project.path(from), specifier)
    }

    /// What `specifier` resolves to when the file `from` imports it.
    fn resolve(&self, from: &str, specifier: &str) -> Resolved {
        self.resolve_ignoring(from, specifier, &[])
    }

    /// The name of the file that `specifier` resolves to, which must be in
    /// the project.
    fn file(&self, from: &str, specifier: &str) -> String {
        match self.resolve(from, specifier) {
            Resolved::Internal(name) => name,
            other => panic!("{from}: {specifier} resolved to {other:?}"),
        }
    }

    /// The package that `specifier` resolves to, which a build installed.
    fn package(&self, from: &str, specifier: &str) -> String {
        match self.resolve(from, specifier) {
            Resolved::External { package, found: true } => package,
            other => panic!("{from}: {specifier} resolved to {other:?}"),
        }
    }

    /// Why `specifier` does not resolve, which it must not.
    fn failure(&self, from: &str, specifier: &str) -> String {
        match self.resolve(from, specifier) {
            Resolved::Unresolved(reason) => reason.to_string(),
            other => panic!("{from}: {specifier} resolved to {other:?}"),
        }
    }
}

#[test]
fn a_module_of_the_component_is_its_file() {
    let fixture = Fixture::built();
    assert_eq!(
        fixture.file("core/src/Acme/Core.hs", "Acme.Core.Types"),
        "core/src/Acme/Core/Types.hs"
    );
}

#[test]
fn a_module_of_a_library_of_the_project_is_its_file() {
    let fixture = Fixture::built();
    assert_eq!(fixture.file("api/src/Acme/Api.hs", "Acme.Core"), "core/src/Acme/Core.hs");
    // An executable and a test suite depend on their own package's library.
    assert_eq!(fixture.file("api/app/Main.hs", "Acme.Api"), "api/src/Acme/Api.hs");
    assert_eq!(fixture.file("api/test/Spec.hs", "Acme.Core.Types"), "core/src/Acme/Core/Types.hs");
}

#[test]
fn a_module_of_an_installed_library_is_its_package() {
    let fixture = Fixture::built();
    assert_eq!(fixture.package("core/src/Acme/Core.hs", "Data.List.Split"), "split");
    assert_eq!(fixture.package("core/src/Acme/Core.hs", "Data.Map.Strict"), "containers");
    assert_eq!(fixture.package("core/src/Acme/Core.hs", "Data.Maybe"), "base");
    // `base` re-exports `GHC.Num.Integer` from `ghc-bignum`, which the file doesn't
    // depend on.
    assert_eq!(fixture.package("core/src/Acme/Core.hs", "GHC.Num.Integer"), "base");
}

#[test]
fn a_module_of_a_library_that_the_component_forgot_is_still_found() {
    let fixture = Fixture::built();
    // `acme-api` doesn't depend on `split`, which GHC would say, but the module is
    // its.
    assert_eq!(fixture.package("api/src/Acme/Api.hs", "Data.List.Split"), "split");
}

#[test]
fn a_file_that_no_component_compiles_sees_every_library() {
    let fixture = Fixture::built();
    assert_eq!(fixture.file("scripts/Migrate.hs", "Acme.Core"), "core/src/Acme/Core.hs");
    assert_eq!(fixture.package("scripts/Migrate.hs", "Data.Map.Strict"), "containers");
}

#[test]
fn the_modules_that_cabal_writes_are_left_out() {
    let fixture = Fixture::built();
    let reason = message!(haskell_generated_module);
    assert_eq!(
        fixture.resolve("core/src/Acme/Core.hs", "Paths_acme_core"),
        Resolved::ignored(reason.clone())
    );
    assert_eq!(
        fixture.resolve("api/app/Main.hs", "PackageInfo_acme_api"),
        Resolved::ignored(reason)
    );
}

#[test]
fn an_import_of_a_boot_file_is_of_the_boot_file() {
    let fixture = Fixture::built();
    let from = "core/src/Acme/Core.hs";
    assert_eq!(
        fixture.file(from, "{-# SOURCE #-} Acme.Core.Types"),
        "core/src/Acme/Core/Types.hs-boot"
    );
    // A module without a boot file is its source.
    assert_eq!(
        fixture.file("api/src/Acme/Api.hs", "{-# SOURCE #-} Acme.Core"),
        "core/src/Acme/Core.hs"
    );
}

#[test]
fn an_import_that_names_a_package_looks_only_in_it() {
    let fixture = Fixture::built();
    let from = "prelude/src/Acme/Prelude.hs";
    assert_eq!(fixture.package(from, "\"containers\" Data.Map"), "containers");
    assert_eq!(fixture.package(from, "\"legacy-containers\" Data.Map"), "legacy-containers");
    assert_eq!(
        fixture.file("core/src/Acme/Core.hs", "\"this\" Acme.Core.Types"),
        "core/src/Acme/Core/Types.hs"
    );
    // A package that no build installed is still the one that the import names.
    let resolved = fixture.resolve(from, "\"aeson\" Data.Aeson");
    assert_eq!(resolved, Resolved::External { package: "aeson".into(), found: false });
    // But a package of the project's that doesn't have the module is not.
    let missing = message!(haskell_no_module);
    assert_eq!(fixture.failure(from, "\"acme-core\" Acme.Missing"), missing);
    assert_eq!(fixture.failure("core/src/Acme/Core.hs", "\"this\" Acme.Missing"), missing);
}

#[test]
fn a_module_that_two_libraries_have_is_ambiguous() {
    let fixture = Fixture::built();
    assert_eq!(
        fixture.failure("prelude/src/Acme/Prelude.hs", "Data.Map"),
        message!(haskell_ambiguous_module, packages = "containers, legacy-containers")
    );
}

#[test]
fn a_component_sees_its_dependencies_through_its_mixins() {
    let fixture = Fixture::built();
    let tool = "prelude/tool/Tool.hs";
    // The tool hides `base`'s Prelude, and sees `relude`'s `Relude` as its own.
    assert_eq!(fixture.package(tool, "Prelude"), "relude");
    assert_eq!(fixture.package(tool, "Data.Maybe"), "base");
    assert_eq!(fixture.file(tool, "Acme.Internal"), "prelude/internal/Acme/Internal.hs");
}

#[test]
fn a_module_that_a_library_of_the_project_re_exports_is_where_it_is_from() {
    let fixture = Fixture::built();
    assert_eq!(fixture.package("prelude/tool/Tool.hs", "Acme.Map"), "containers");
}

#[test]
fn a_module_that_no_library_has_does_not_resolve() {
    let built = Fixture::built();
    assert_eq!(built.failure("core/src/Acme/Core.hs", "Data.Aeson"), message!(haskell_no_module));
    let unbuilt = Fixture::unbuilt();
    assert_eq!(
        unbuilt.failure("core/src/Acme/Core.hs", "Data.List.Split"),
        message!(haskell_no_build)
    );
    // The compiler's own libraries need no build.
    assert_eq!(unbuilt.package("core/src/Acme/Core.hs", "Data.Map.Strict"), "containers");
}

#[test]
fn unresolved_specifiers_that_match_a_pattern_are_ignored() {
    let fixture = Fixture::built();
    let patterns = [NamePattern::new("Data.Aeson*").unwrap()];
    // The patterns match the module, whatever comes before it.
    for specifier in ["Data.Aeson.Types", "{-# SOURCE #-} Data.Aeson.Types"] {
        let resolved = fixture.resolve_ignoring("core/src/Acme/Core.hs", specifier, &patterns);
        assert!(matches!(resolved, Resolved::Ignored(_)), "{specifier}: {resolved:?}");
    }
}

#[test]
fn a_file_does_not_import_itself() {
    let fixture = Fixture::built();
    assert_eq!(
        fixture.resolve("core/src/Acme/Core.hs", "Acme.Core"),
        Resolved::ignored(message!(haskell_importing_file))
    );
}
