//! Resolving the modules that Python files import to the files that hold
//! them.
//!
//! A specifier is a module path with `.`, such as `acme.api.handlers`, and
//! `from a import b` is written `a.b`, so that `b` is a module if there is
//! one, and a name in `a` otherwise. A relative specifier starts with a `.`
//! for the importing file's package, and one more for each package above it.
//!
//! An absolute specifier starts at a root, as on `sys.path`. Its first name
//! is a module or a regular package of the project in one of the roots, or
//! else of the standard library, or else a namespace package of the project,
//! or else a third-party package. The path leads through the packages to the
//! file of the last module on it, and what follows is names in that module.

#[cfg(test)]
mod tests;

use std::{path::Path, sync::Arc};

use zenith_diagnostics::message;
use zenith_lang::{Resolved, UnresolvedReason};
use zenith_utils::{fs::FileSystem, glob::NamePattern, path::relative_posix};

use crate::{
    modules::{Directories, Module, Roots},
    stdlib::is_standard,
};

/// Resolves the imports of the Python files of a project, with the `roots`
/// that were found in it, which are shared between threads.
pub(crate) struct Resolver<'a> {
    /// The canonical project root.
    root: &'a Path,

    /// The roots that absolute imports start from.
    roots: &'a Roots,

    /// What the directories hold, as far as resolving has read them.
    directories: Directories,

    /// Patterns of the module paths that are expected not to resolve.
    ignore_unresolved: &'a [NamePattern],
}

impl<'a> Resolver<'a> {
    /// A resolver of the project at `root` in `file_system`, which must be
    /// canonical there, which expects the module paths that match
    /// `ignore_unresolved` not to resolve.
    pub(crate) fn new(
        root: &'a Path,
        file_system: Arc<dyn FileSystem>,
        roots: &'a Roots,
        ignore_unresolved: &'a [NamePattern],
    ) -> Self {
        Self { root, roots, directories: Directories::new(file_system), ignore_unresolved }
    }

    /// What `specifier`, a module path written with `.`, refers to when the
    /// file at `path` imports it.
    pub(crate) fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        self.resolve_specifier(path, specifier)
            .ignoring_unresolved(specifier, self.ignore_unresolved)
    }

    /// Resolve `specifier`, without applying `ignore_unresolved`.
    fn resolve_specifier(&self, path: &Path, specifier: &str) -> Resolved {
        let rest = specifier.trim_start_matches('.');
        let dots = specifier.len() - rest.len();
        let names: Vec<&str> = rest.split('.').filter(|name| !name.is_empty()).collect();
        match dots {
            0 if names.is_empty() => Resolved::failed(message!(python_no_module)),
            0 => self.absolute(path, &names),
            _ => self.relative(path, dots, &names),
        }
    }

    /// What `names` refer to when the file at `path` imports them with
    /// `dots` leading dots.
    fn relative(&self, path: &Path, dots: usize, names: &[&str]) -> Resolved {
        // 1. One dot is the file's own package, and each other dot the package above.
        let mut package = path.parent().unwrap_or(path);
        for _ in 1..dots {
            match package.parent() {
                Some(parent) if package != self.root => package = parent,
                _ => return Resolved::failed(message!(python_above_root)),
            }
        }

        // 2. A module in the package, or else a name in the package itself. A namespace
        //    package has no file to hold names, so only `from . import *` imports it.
        if let Some(module) = Module::find(&self.directories, package, names) {
            return self.file(&module.file, path);
        }
        match (self.directories.init_file(package), names.first()) {
            (Some(init), _) => self.file(&init, path),
            (None, Some(name)) => Resolved::failed(message!(python_not_in_namespace, name)),
            (None, None) => match relative_posix(self.root, package) {
                Some(dir) => Resolved::Package(dir),
                None => Resolved::unresolved(UnresolvedReason::OutsideRoot(package.to_path_buf())),
            },
        }
    }

    /// What `names`, which start with a top-level module, refer to when the
    /// file at `path` imports them.
    fn absolute(&self, path: &Path, names: &[&str]) -> Resolved {
        let top = names[0];

        // 1. A module of the project, in the roots and then in the directory of the
        //    file's outermost package, as it would be when run from there.
        let mut namespace = None;
        let own = self.directories.package_root(path, self.root);
        let roots = self.roots.having(top, path);
        let extra = (!roots.contains(&own)).then_some(own);
        for root in roots.into_iter().chain(extra) {
            match Module::find(&self.directories, root, names) {
                Some(module) if module.regular => return self.file(&module.file, path),
                Some(module) => namespace = namespace.or(Some(module)),
                None => {}
            }
        }

        // 2. The standard library, which a namespace package doesn't hide.
        if is_standard(top) {
            return Resolved::Builtin(format!("python:{top}"));
        }

        // 3. A module in a namespace package of the project, or else a third-party
        //    package. Its top-level module is the name that it is meant to have, not
        //    one that stands in for an install that wasn't found.
        match namespace {
            Some(module) => self.file(&module.file, path),
            None => Resolved::External { package: top.to_owned(), found: true },
        }
    }

    /// `file`, imported by the file at `importer`, as it is named in the
    /// project. A file does not import itself.
    fn file(&self, file: &Path, importer: &Path) -> Resolved {
        if file == importer {
            return Resolved::ignored(message!(python_importing_file));
        }
        match relative_posix(self.root, file) {
            Some(name) => Resolved::Internal(name),
            None => Resolved::unresolved(UnresolvedReason::OutsideRoot(file.to_path_buf())),
        }
    }
}
