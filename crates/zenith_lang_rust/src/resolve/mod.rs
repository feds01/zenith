//! Resolving the paths that Rust files import to the files that hold them.
//!
//! A path starts with `crate`, `self` or `super`, which are relative to the
//! module of the importing file, or with the name of a crate: a dependency of
//! the importing file's package, its own library, or a crate of the standard
//! library. Otherwise the first segment is a module of the file's own module,
//! as `use` paths may start with one. The path leads through the modules to
//! the file of the last one, and what follows is an item in it. Inline
//! modules and `#[path]` attributes are not followed.

#[cfg(test)]
mod tests;

use std::path::Path;

use zenith_lang::{ResolveFailure, Resolved, UnresolvedReason};
use zenith_utils::path::relative_posix;

use crate::{
    crates::{Crate, Crates, Dependency, STANDARD_CRATES},
    modules::{Position, Root},
};

/// Resolves the imports of the Rust files of a project, with the `crates`
/// that were found in it, which are shared between threads.
pub(crate) struct Resolver<'a> {
    /// The canonical project root.
    root: &'a Path,

    /// The crates of the project.
    crates: &'a Crates,
}

impl<'a> Resolver<'a> {
    /// A resolver of the project at `root`, which must be canonical.
    pub(crate) fn new(root: &'a Path, crates: &'a Crates) -> Self {
        Self { root, crates }
    }

    /// What `specifier`, a path written with `::`, refers to when the file at
    /// `path` imports it.
    pub(crate) fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        // 1. Find the package of the importing file.
        let Some(krate) = self.crates.containing(path) else {
            return failed("the file is in no Cargo package");
        };

        // 2. Split the path. One that starts with `::` names a crate, never a module.
        let (rooted, specifier) = match specifier.strip_prefix("::") {
            Some(rest) => (true, rest),
            None => (false, specifier),
        };
        let segments: Vec<&str> = specifier.split("::").collect();

        // 3. Resolve it from where it starts.
        match segments[0] {
            "crate" | "self" | "super" if !rooted => self.relative(krate, path, &segments),
            name => self.named(krate, path, rooted, name, &segments[1..]),
        }
    }

    /// What `segments`, which start with `crate`, `self` or `super`, refer to
    /// when they are imported by the file at `path`, in `krate`.
    fn relative(&self, krate: &Crate, path: &Path, segments: &[&str]) -> Resolved {
        let Some(Position { root, modules }) = krate.locate(path) else {
            return failed(format!("the file is in no module tree of `{}`", krate.name));
        };

        // 1. Start at the root for `crate`, and at the file's module otherwise.
        let (mut modules, mut rest): (Vec<&str>, _) = match segments {
            ["crate", rest @ ..] => (Vec::new(), rest),
            _ => (modules.iter().map(String::as_str).collect(), segments),
        };

        // 2. Each `self` stays, and each `super` leaves the module.
        while let Some((&step @ ("self" | "super"), tail)) = rest.split_first() {
            if step == "super" && modules.pop().is_none() {
                return failed("`super` goes above the crate root");
            }
            rest = tail;
        }

        modules.extend(rest);
        self.module(&root, &modules, path)
    }

    /// What `rest`, the segments after the first, refer to when `name`, the
    /// first, is not `crate`, `self` or `super`, and the file at `path`, in
    /// `krate`, imports them.
    fn named(
        &self,
        krate: &Crate,
        path: &Path,
        rooted: bool,
        name: &str,
        rest: &[&str],
    ) -> Resolved {
        // 1. A crate: a dependency, the crate's own library, or the standard library.
        match krate.dependencies.get(name) {
            Some(Dependency::Internal(index)) => {
                return self.module(&self.crates.get(*index).library(), rest, path);
            }
            Some(Dependency::External(package)) => {
                return Resolved::External { package: package.clone(), found: true };
            }
            None if STANDARD_CRATES.contains(&name) => {
                return Resolved::Builtin(format!("rust:{name}"));
            }
            None => {}
        }

        // 2. A module of the file's own module, which `use` paths may start with. A
        //    name that starts with a capital is a type in scope, such as an enum whose
        //    variants are imported.
        if name.starts_with(char::is_uppercase) {
            return ignored("a type in scope, not a crate or module");
        }
        if !rooted && let Some(Position { root, modules }) = krate.locate(path) {
            let mut all: Vec<&str> = modules.iter().map(String::as_str).collect();
            let own = all.len();
            all.push(name);
            all.extend(rest);
            if let Some((file, depth)) = root.module_file(&all)
                && depth > own
            {
                return self.file(&file, path);
            }
        }
        failed(format!("`{name}` is not a crate, or a module here"))
    }

    /// The file of the module that `modules` name in the tree of `root`, for
    /// an import in the file at `importer`.
    fn module(&self, root: &Root, modules: &[&str], importer: &Path) -> Resolved {
        match root.module_file(modules) {
            Some((file, _)) => self.file(&file, importer),
            None => failed("the root module of the crate is not a file"),
        }
    }

    /// `file`, imported by the file at `importer`, as it is named in the
    /// project. A file does not import itself.
    fn file(&self, file: &Path, importer: &Path) -> Resolved {
        if file == importer {
            return ignored("the path is in the importing file");
        }
        match relative_posix(self.root, file) {
            Some(name) => Resolved::Internal(name),
            None => Resolved::unresolved(UnresolvedReason::OutsideRoot(file.to_path_buf())),
        }
    }
}

/// A path that can't be resolved, for this reason.
fn failed(reason: impl Into<String>) -> Resolved {
    Resolved::unresolved(UnresolvedReason::Failed(ResolveFailure::new(reason)))
}

/// A path that is left out, for this reason.
fn ignored(reason: &'static str) -> Resolved {
    Resolved::Ignored(Box::new(UnresolvedReason::Failed(ResolveFailure::new(reason))))
}
