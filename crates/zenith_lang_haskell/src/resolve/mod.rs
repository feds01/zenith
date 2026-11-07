//! Resolving the modules that Haskell files import, to the files of the
//! project that hold them, or to the packages outside it that do.
//!
//! A module is looked for as GHC looks for it as it builds the component
//! that compiles the importing file, as the file's package describes it:
//!
//! 1. among the component's own modules, in its source directories, or those
//!    that Cabal writes for it as it builds, such as `Paths_acme`, which have
//!    no file;
//! 2. among the modules of the libraries that the component depends on, as it
//!    sees them: a library of the project's, whose module is a file of it, or
//!    one that a build installed, which is its package;
//! 3. among the modules of every library of the project, then of every one that
//!    the build installed, for a file that no component compiles, such as a
//!    script, or a module of a library that the component forgot to depend on.
//!
//! An import that names a package looks only in it, and resolves to it
//! when no library that the build installed says more. An import of a boot
//! file is of the module's `.hs-boot` file, if it has one. A module that more
//! than one library has is ambiguous, as GHC finds it.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use zenith_diagnostics::message;
use zenith_lang::{Resolved, UnresolvedReason};
use zenith_utils::{
    fs::{FileSystem, ListingCache},
    glob::NamePattern,
    path::relative_posix,
};

use crate::{
    packages::{Build, Component, Package, Packages},
    parse::Specifier,
};

/// How deep re-exports are followed, from a library to the one that it
/// re-exports a module of, which no real project nests anywhere near.
const REEXPORT_DEPTH: usize = 8;

/// The extensions of the file of a module, in the order that they are looked
/// for: Haskell's own, plain and literate, those of signatures, and those of
/// the sources that a preprocessor makes the module of, such as `hsc2hs`'s
/// `.hsc`, Alex's `.x` and Happy's `.y`.
const EXTENSIONS: &[&str] = &["hs", "lhs", "hsig", "lhsig", "hsc", "chs", "x", "y", "ly"];

/// The extensions of the boot file of a module, which an import with
/// `{-# SOURCE #-}` is of, and which are looked for first.
const BOOT_EXTENSIONS: &[&str] = &["hs-boot", "lhs-boot"];

/// Resolves the imports of the Haskell files of a project, with the
/// `packages` that were found in it, which are shared between threads.
pub(crate) struct Resolver<'a> {
    /// The canonical project root.
    root: &'a Path,

    /// The packages of the project, and what its builds installed.
    packages: &'a Packages,

    /// The project's file system, which lists each directory that resolving
    /// looks in once. Names are compared exactly, as GHC compares them, even
    /// where the file system ignores case.
    file_system: ListingCache<'a>,

    /// Patterns of the specifiers that are expected not to resolve.
    ignore_unresolved: &'a [NamePattern],
}

/// Where a module was found.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Found<'a> {
    /// A file of the project.
    File(PathBuf),

    /// A library that a build installed, of this package.
    Package(&'a str),
}

impl<'a> Resolver<'a> {
    /// A resolver of the project at `root` in `file_system`, which must be
    /// canonical there, with its `packages`, which expects the specifiers that
    /// match `ignore_unresolved` not to resolve.
    pub(crate) fn new(
        root: &'a Path,
        file_system: &'a dyn FileSystem,
        packages: &'a Packages,
        ignore_unresolved: &'a [NamePattern],
    ) -> Self {
        let file_system = ListingCache::new(file_system);
        Self { root, packages, file_system, ignore_unresolved }
    }

    /// What `specifier` refers to when the file at `path` imports it. The
    /// patterns of `ignore_unresolved` match its module.
    pub(crate) fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        let import = Specifier::parse(specifier);
        self.resolve_import(path, import).ignoring_unresolved(import.module, self.ignore_unresolved)
    }

    /// What `import` refers to, without applying `ignore_unresolved`.
    fn resolve_import(&self, path: &Path, import: Specifier<'_>) -> Resolved {
        let package = self.packages.containing(path);
        let components: Vec<&Component> =
            package.map_or_else(Vec::new, |package| package.components_of(path).collect());
        let build = self.packages.build_of(path);

        // 1. A module of the file's own components, unless the import names another
        //    package. GHC takes `this` for the importing package.
        if let Some(package) = package
            && import.package.is_none_or(|name| name == package.name || name == "this")
        {
            let generated = package.generates(import.module)
                || components.iter().any(|component| {
                    component.generated.iter().any(|module| module == import.module)
                });
            if generated {
                return Resolved::ignored(message!(haskell_generated_module));
            }
            let dirs = components.iter().flat_map(|component| &component.source_dirs);
            if let Some(file) = self.module(dirs, import.module, import.boot) {
                return self.file(&file, path);
            }
        }

        // 2. A module of the libraries that the components depend on, as they see them.
        let mut found = Vec::new();
        for component in &components {
            for dependency in &component.dependencies {
                if import.package.is_some_and(|name| name != dependency.package) {
                    continue;
                }
                let Some(module) = component.original(dependency, import.module) else {
                    continue;
                };
                let library = dependency.library.as_deref();
                found.extend(self.in_library(
                    build,
                    &dependency.package,
                    library,
                    module,
                    import.boot,
                    0,
                ));
            }
        }

        // 3. A module of any library of the project, or else of the build.
        if found.is_empty() {
            found = self.anywhere(build, import);
        }
        found.sort();
        found.dedup();

        match (found.as_slice(), import.package) {
            ([found], _) => match found {
                Found::File(file) => self.file(file, path),
                Found::Package(package) => {
                    Resolved::External { package: (*package).to_owned(), found: true }
                }
            },
            // A package of the project's that doesn't have the module is not one that a
            // build may have installed.
            ([], Some(package)) if package == "this" || self.packages.named(package).is_some() => {
                Resolved::failed(message!(haskell_no_module))
            }
            ([], Some(package)) => Resolved::External { package: package.to_owned(), found: false },
            ([], None) if build.root.is_some() => Resolved::failed(message!(haskell_no_module)),
            ([], None) => Resolved::failed(message!(haskell_no_build)),
            (found, _) => {
                let packages =
                    found.iter().map(|found| self.package_of(found)).collect::<Vec<_>>().join(", ");
                Resolved::failed(message!(haskell_ambiguous_module, packages))
            }
        }
    }

    /// Where `module` is in the library named `library` of `package`, or its
    /// main library for `None`: the project's, if it has the package, or else
    /// the one that `build` installed. A module that a library of the project
    /// re-exports is looked for in the libraries that it depends on, `depth`
    /// re-exports deep.
    fn in_library(
        &self,
        build: &'a Build,
        package: &'a str,
        library: Option<&str>,
        module: &str,
        boot: bool,
        depth: usize,
    ) -> Vec<Found<'a>> {
        let Some(local) = self.packages.named(package) else {
            let installed = build.library(package, library);
            let exposes = installed
                .is_some_and(|installed| installed.exposed.iter().any(|exposed| exposed == module));
            return exposes.then_some(Found::Package(package)).into_iter().collect();
        };
        let Some(component) = local.library(library) else {
            return Vec::new();
        };
        if component.may_expose(module)
            && let Some(file) = self.module(&component.source_dirs, module, boot)
        {
            return vec![Found::File(file)];
        }
        match component.reexport(module) {
            Some(reexport) if depth < REEXPORT_DEPTH => (component.dependencies.iter())
                .filter(|dependency| {
                    reexport.package.as_ref().is_none_or(|name| *name == dependency.package)
                })
                .flat_map(|dependency| {
                    let library = dependency.library.as_deref();
                    self.in_library(
                        build,
                        &dependency.package,
                        library,
                        &reexport.original,
                        false,
                        depth + 1,
                    )
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Where `import` is among every library of the project, or else among
    /// those that `build` installed.
    fn anywhere(&self, build: &'a Build, import: Specifier<'_>) -> Vec<Found<'a>> {
        let named = |package: &str| import.package.is_none_or(|name| name == package);
        let local: Vec<Found<'a>> = (self.packages.exposing(import.module))
            .filter(|(package, _)| named(&package.name))
            .filter_map(|(_, component)| {
                self.module(&component.source_dirs, import.module, import.boot)
            })
            .map(Found::File)
            .collect();
        if !local.is_empty() {
            return local;
        }
        (build.exposing(import.module))
            .filter(|installed| named(&installed.package))
            .map(|installed| Found::Package(&installed.package))
            .collect()
    }

    /// The file of `module`, such as `Data.Map.Strict`, in the first of
    /// `dirs` that has one: its boot file, if `boot` and it has one, or else
    /// its source.
    fn module<'d>(
        &self,
        dirs: impl IntoIterator<Item = &'d PathBuf>,
        module: &str,
        boot: bool,
    ) -> Option<PathBuf> {
        let (parents, name) = module.rsplit_once('.').unwrap_or(("", module));
        let boot_extensions = match boot {
            true => BOOT_EXTENSIONS,
            false => &[],
        };
        dirs.into_iter().find_map(|dir| {
            let parts = parents.split('.').filter(|part| !part.is_empty());
            let dir = parts.fold(dir.clone(), |dir, part| dir.join(part));
            (boot_extensions.iter().chain(EXTENSIONS))
                .map(|extension| dir.join(format!("{name}.{extension}")))
                .find(|file| self.file_system.is_listed(file))
        })
    }

    /// The name of the package that `found` is in, for a message.
    fn package_of(&self, found: &Found<'_>) -> String {
        match found {
            Found::Package(package) => (*package).to_owned(),
            Found::File(file) => self
                .packages
                .containing(file)
                .map_or_else(String::new, |package: &Package| package.name.clone()),
        }
    }

    /// `file`, imported by the file at `importer`, as it is named in the
    /// project. A file does not import itself.
    fn file(&self, file: &Path, importer: &Path) -> Resolved {
        if file == importer {
            return Resolved::ignored(message!(haskell_importing_file));
        }
        match relative_posix(self.root, file) {
            Some(name) => Resolved::Internal(name),
            None => Resolved::unresolved(UnresolvedReason::OutsideRoot(file.to_path_buf())),
        }
    }
}
