//! Tests of the packages: their `.cabal` descriptions, the entries of
//! package databases, and what a component sees of its dependencies. The
//! descriptions and entries of the acme project, and of `base`,
//! `containers` and `split`, are those of a real build with cabal.

use std::path::{Path, PathBuf};

use zenith_utils::path::normalize_path;

use super::{
    fields::{self, Item},
    *,
};

/// The path `path` in the project at `/repo`, absolute as a project's
/// directories are, so on Windows on the drive of the current directory.
fn repo(path: &str) -> PathBuf {
    normalize_path(Path::new("/repo").join(path))
}

/// The package that the description `text`, in `/repo/<dir>`, describes.
fn describe(dir: &str, text: &str) -> Package {
    Package::describe(&repo(dir), text).expect("a package")
}

/// The component of `package` of `kind`.
fn component(package: &Package, kind: ComponentKind) -> &Component {
    package
        .components
        .iter()
        .find(|component| component.kind == kind)
        .unwrap_or_else(|| panic!("{kind:?}"))
}

/// The packages and libraries of `dependencies`, as `package` or
/// `package:library`.
fn dependencies(component: &Component) -> Vec<String> {
    (component.dependencies.iter())
        .map(|dependency| match &dependency.library {
            Some(library) => format!("{}:{library}", dependency.package),
            None => dependency.package.clone(),
        })
        .collect()
}

#[test]
fn reads_fields_sections_and_the_lines_that_continue_them() {
    let items = fields::parse(
        "Name: acme\n-- a comment\nlibrary\n  exposed-modules:\n    A\n    B\n  if os(windows)\n    \
         build-depends: Win32\nexecutable acme\n  main-is: Main.hs\n",
    );
    assert_eq!(fields::fields(&items, "name").collect::<Vec<_>>(), ["acme"]);
    let sections: Vec<_> = fields::sections(&items)
        .map(|section| (section.name.as_str(), section.arguments.as_str()))
        .collect();
    assert_eq!(sections, [("library", ""), ("executable", "acme")]);
    let library = fields::sections(&items).next().unwrap();
    assert_eq!(fields::fields(&library.items, "exposed-modules").collect::<Vec<_>>(), ["\nA\nB"]);
    let Some(Item::Section(conditional)) = library.items.last() else { panic!("{library:?}") };
    assert_eq!((conditional.name.as_str(), conditional.arguments.as_str()), ("if", "os(windows)"));
}

#[test]
fn reads_the_words_of_a_list() {
    assert_eq!(fields::words("src, \"generated src\"\n  test"), ["src", "generated src", "test"]);
    assert_eq!(fields::words("  "), Vec::<String>::new());
}

#[test]
fn describes_the_components_of_a_package() {
    let core = describe("core", include_str!("fixtures/acme-core.cabal"));
    assert_eq!(core.name, "acme-core");
    let library = component(&core, ComponentKind::Library(None));
    assert_eq!(library.source_dirs, [repo("core/src")]);
    assert_eq!(library.exposed, ["Acme.Core", "Acme.Core.Types"]);
    // `base` comes from the common stanza that the library imports.
    assert_eq!(dependencies(library), ["base", "containers", "split"]);

    let api = describe("api", include_str!("fixtures/acme-api.cabal"));
    let kinds: Vec<_> = api.components.iter().map(|component| component.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            ComponentKind::Library(None),
            ComponentKind::Executable("acme".into()),
            ComponentKind::TestSuite("spec".into())
        ]
    );
    let executable = component(&api, ComponentKind::Executable("acme".into()));
    assert_eq!(
        (executable.main.as_deref(), executable.source_dirs.as_slice()),
        (Some("Main.hs"), &[repo("api/app")][..])
    );
    assert_eq!(dependencies(executable), ["base", "acme-api"]);
}

#[test]
fn reads_every_field_of_every_branch_and_stanza() {
    let full = describe("full", include_str!("fixtures/acme-full.cabal"));
    assert_eq!(full.name, "acme-full");
    let library = component(&full, ComponentKind::Library(None));
    let dirs: Vec<_> =
        library.source_dirs.iter().map(|dir| dir.strip_prefix(repo("full")).unwrap()).collect();
    assert_eq!(dirs, [Path::new("src"), Path::new("generated src"), Path::new("windows")]);
    assert_eq!(library.exposed, ["Acme.Full", "Acme.Full.Types", "Acme.Full.Store"]);
    assert_eq!(library.generated, ["Acme.Full.Version"]);
    assert_eq!(
        dependencies(library),
        ["base", "containers", "acme-full:internal", "acme-full:other", "Win32", "unix"]
    );
    assert_eq!(
        library.reexported,
        [
            Reexport { module: "Acme.Map".into(), original: "Data.Map".into(), package: None },
            Reexport {
                module: "Data.Set".into(),
                original: "Data.Set".into(),
                package: Some("containers".into())
            },
        ]
    );
    assert_eq!(
        library.mixins,
        [
            Mixin {
                package: "base".into(),
                library: None,
                visible: Visible::Hiding(vec!["Prelude".into()])
            },
            Mixin {
                package: "relude".into(),
                library: None,
                visible: Visible::Only(vec![
                    ("Relude".into(), "Prelude".into()),
                    ("Relude.Extra".into(), "Relude.Extra".into())
                ])
            },
            Mixin { package: "text".into(), library: None, visible: Visible::All },
        ]
    );

    let internal = component(&full, ComponentKind::Library(Some("internal".into())));
    assert_eq!(internal.exposed, ["Acme.Internal"]);
    let spec = component(&full, ComponentKind::TestSuite("spec".into()));
    assert_eq!(dependencies(spec), ["base", "acme-full:internal"]);
    // A component that names no source directory has its package's.
    let bench = component(&full, ComponentKind::Benchmark("bench".into()));
    assert_eq!(bench.source_dirs, [repo("full")]);
}

#[test]
fn a_description_without_a_name_is_no_package() {
    assert_eq!(Package::describe(Path::new("/repo"), "library\n  exposed-modules: A\n"), None);
}

#[test]
fn source_directories_are_without_dots() {
    let text = "name: flat\nlibrary\n  hs-source-dirs: ../shared/src ./src .\n";
    let flat = describe("flat", text);
    let library = component(&flat, ComponentKind::Library(None));
    let dirs = ["shared/src", "flat/src", "flat"].map(repo);
    assert_eq!(library.source_dirs, dirs);
}

#[test]
fn a_component_sees_the_modules_that_its_mixins_leave() {
    let full = describe("full", include_str!("fixtures/acme-full.cabal"));
    let library = component(&full, ComponentKind::Library(None));
    let main = |package: &str| Dependency { package: package.into(), library: None };
    assert_eq!(library.original(&main("base"), "Prelude"), None);
    assert_eq!(library.original(&main("base"), "Data.List"), Some("Data.List"));
    assert_eq!(library.original(&main("relude"), "Prelude"), Some("Relude"));
    assert_eq!(library.original(&main("relude"), "Relude"), None);
    assert_eq!(library.original(&main("text"), "Data.Text"), Some("Data.Text"));
    assert_eq!(library.original(&main("containers"), "Data.Map"), Some("Data.Map"));
}

#[test]
fn a_mixin_of_a_library_renames_its_modules_alone() {
    let text = "name: acme\nlibrary\n  build-depends: acme-vendor, acme-vendor:internal\n  mixins: \
                acme-vendor:internal (Vendor.Internal as Acme.Vendor)\n";
    let acme = describe("acme", text);
    let library = component(&acme, ComponentKind::Library(None));
    let renamed = vec![("Vendor.Internal".into(), "Acme.Vendor".into())];
    let internal = Some("internal".to_owned());
    assert_eq!(
        library.mixins,
        [Mixin {
            package: "acme-vendor".into(),
            library: internal.clone(),
            visible: Visible::Only(renamed)
        }]
    );
    let vendor = |library| Dependency { package: "acme-vendor".into(), library };
    assert_eq!(library.original(&vendor(internal), "Acme.Vendor"), Some("Vendor.Internal"));
    assert_eq!(library.original(&vendor(None), "Vendor"), Some("Vendor"));
}

#[test]
fn reads_the_entries_of_package_databases() {
    let split = Installed::parse(include_str!("fixtures/split.conf")).unwrap();
    assert_eq!(split.package, "split");
    assert_eq!(
        split.id,
        "split-0.2.5.1-7565964c8d9feac3393ebb647a76ad58ced9d2f6ca6b144daaaa33fd28545987"
    );
    assert_eq!(split.exposed, ["Data.List.Split", "Data.List.Split.Internals"]);

    let core = Installed::parse(include_str!("fixtures/acme-core-inplace.conf")).unwrap();
    assert_eq!(
        (core.id.as_str(), core.exposed.as_slice()),
        ("acme-core-0.1.0.0-inplace", &["Acme.Core".to_owned(), "Acme.Core.Types".to_owned()][..])
    );

    // `base` re-exports modules of `ghc-bignum`, which are its own to who imports
    // them.
    let base = Installed::parse(include_str!("fixtures/base.conf")).unwrap();
    assert!(base.exposed.iter().any(|module| module == "Data.List"));
    assert!(base.exposed.iter().any(|module| module == "GHC.Num.Integer"));
    assert!(!base.exposed.iter().any(|module| module == "from" || module.contains(':')));

    let vendor = Installed::parse(include_str!("fixtures/sublibrary.conf")).unwrap();
    assert_eq!(
        (vendor.package.as_str(), vendor.library.as_deref()),
        ("acme-vendor", Some("internal"))
    );
}

#[test]
fn a_build_chooses_the_first_library_of_each_package() {
    let parse = |text: &str| Installed::parse(text).unwrap();
    let mut local = parse(include_str!("fixtures/split.conf"));
    local.id = "split-local".into();
    let build = Build::new(None, vec![local, parse(include_str!("fixtures/split.conf"))]);
    assert_eq!(build.library("split", None).map(|split| split.id.as_str()), Some("split-local"));
    let exposing: Vec<_> =
        build.exposing("Data.List.Split").map(|split| split.id.as_str()).collect();
    assert_eq!(exposing, ["split-local"]);
    assert_eq!(build.library("split", Some("internal")), None);
}

#[test]
fn knows_the_files_that_only_tests_compile() {
    let api = describe("api", include_str!("fixtures/acme-api.cabal"));
    assert!(api.compiles_only_tests(&repo("api/test/Spec.hs")));
    assert!(!api.compiles_only_tests(&repo("api/src/Acme/Api.hs")));
    assert!(!api.compiles_only_tests(&repo("api/Setup.hs")));
}

#[test]
fn a_component_compiles_its_modules_and_its_main_module() {
    // The library's source directory is the package's own, which holds the
    // others' too.
    let text = "name: flat\nlibrary\n  exposed-modules: Flat\nexecutable acme-tool\n  \
                hs-source-dirs: app\n  main-is: acme-tool.hs\ntest-suite spec\n  hs-source-dirs: \
                test\n  main-is: Spec.hs\n";
    let flat = describe("flat", text);
    let compiling = |path: &str| -> Vec<ComponentKind> {
        let path = repo("flat").join(path);
        flat.components_of(&path).map(|component| component.kind.clone()).collect()
    };
    assert_eq!(compiling("Flat.hs"), [ComponentKind::Library(None)]);
    assert_eq!(compiling("Flat/Internal.lhs"), [ComponentKind::Library(None)]);
    assert_eq!(compiling("app/acme-tool.hs"), [ComponentKind::Executable("acme-tool".into())]);
    assert_eq!(compiling("test/Spec.hs"), [ComponentKind::TestSuite("spec".into())]);
    assert_eq!(compiling("test/Acme/Helpers.hs"), [ComponentKind::TestSuite("spec".into())]);
    assert_eq!(compiling("scripts/migrate.hs"), []);
    assert!(flat.compiles_only_tests(&repo("flat/test/Spec.hs")));
}

#[test]
fn knows_the_modules_that_cabal_writes() {
    let core = describe("core", include_str!("fixtures/acme-core.cabal"));
    assert!(core.generates("Paths_acme_core"));
    assert!(core.generates("PackageInfo_acme_core"));
    assert!(!core.generates("Paths_acme_cor"));
    assert!(!core.generates("Acme.Core"));
}

#[test]
fn packages_are_the_workspace_packages() {
    let packages = Packages::new(
        vec![
            describe("api", include_str!("fixtures/acme-api.cabal")),
            describe("", include_str!("fixtures/acme-full.cabal")),
        ],
        Vec::new(),
        Build::default(),
    );
    let workspace = packages.workspace(&repo(""));
    let names: Vec<_> = workspace
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package.dir.as_str()))
        .collect();
    assert_eq!(names, [("acme-api", "api")]);
    assert_eq!(workspace.root_package.map(|package| package.name), Some("acme-full".into()));
}
