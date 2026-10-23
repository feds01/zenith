//! Resolving the paths that Rust files import to the files that hold them.
//!
//! A path starts with `crate`, `self` or `super`, which are relative to the
//! module of the importing file, or with the name of a crate: a dependency of
//! the importing file's package, its own library, or a crate of the standard
//! library. Otherwise the first segment is a module of the file's own module,
//! as `use` paths may start with one. The path leads through the modules to
//! the file of the last one, and what follows is an item in it. Inline
//! modules and `#[path]` attributes are not followed. A path that starts with
//! the name of a tool, such as `clippy::too_many_lines` in an `#[allow]`, is
//! the tool's attribute or lint, not an import.

#[cfg(test)]
mod tests;

use std::path::Path;

use zenith_diagnostics::message;
use zenith_lang::{Resolved, UnresolvedReason};
use zenith_utils::{glob::NamePattern, path::relative_posix};

use crate::{
    crates::{Crate, Crates, Dependency, STANDARD_CRATES, Target},
    modules::Position,
};

/// The tools whose attributes and lints the compiler knows, such as
/// `#[rustfmt::skip]` and `#[allow(clippy::too_many_lines)]`.
const TOOLS: &[&str] = &["clippy", "diagnostic", "miri", "rust_analyzer", "rustdoc", "rustfmt"];

/// Resolves the imports of the Rust files of a project, with the `crates`
/// that were found in it, which are shared between threads.
pub(crate) struct Resolver<'a> {
    /// The canonical project root.
    root: &'a Path,

    /// The crates of the project.
    crates: &'a Crates,

    /// Patterns of the paths that are expected not to resolve.
    ignore_unresolved: &'a [NamePattern],
}

impl<'a> Resolver<'a> {
    /// A resolver of the project at `root`, which must be canonical, which
    /// expects the paths that match `ignore_unresolved` not to resolve.
    pub(crate) fn new(
        root: &'a Path,
        crates: &'a Crates,
        ignore_unresolved: &'a [NamePattern],
    ) -> Self {
        Self { root, crates, ignore_unresolved }
    }

    /// What `specifier`, a path written with `::`, refers to when the file at
    /// `path` imports it.
    pub(crate) fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        self.resolve_specifier(path, specifier)
            .ignoring_unresolved(specifier, self.ignore_unresolved)
    }

    /// Resolve `specifier`, without applying `ignore_unresolved`.
    fn resolve_specifier(&self, path: &Path, specifier: &str) -> Resolved {
        // 1. Find the package of the importing file.
        let Some(krate) = self.crates.containing(path) else {
            return Resolved::failed(message!(rust_no_package));
        };

        // 2. Split the path. One that starts with `::` names a crate, never a module.
        let (rooted, specifier) = match specifier.strip_prefix("::") {
            Some(rest) => (true, rest),
            None => (false, specifier),
        };
        // A raw identifier, such as `r#false`, names the module `false`.
        let segments: Vec<&str> = specifier
            .split("::")
            .map(|segment| segment.strip_prefix("r#").unwrap_or(segment))
            .collect();

        // 3. Resolve it from where it starts.
        match segments[0] {
            "crate" | "self" | "super" if !rooted => self.relative(krate, path, &segments),
            name => self.named(krate, path, rooted, name, &segments[1..]),
        }
    }

    /// What `segments`, which start with `crate`, `self` or `super`, refer to
    /// when they are imported by the file at `path`, in `krate`.
    fn relative(&self, krate: &Crate, path: &Path, segments: &[&str]) -> Resolved {
        let Some(Position { target, modules }) = krate.locate(path) else {
            return Resolved::failed(message!(rust_no_module_tree, name = krate.name));
        };

        // 1. Start at the root for `crate`, and at the file's module otherwise.
        let (mut modules, mut rest): (Vec<&str>, _) = match segments {
            ["crate", rest @ ..] => (Vec::new(), rest),
            _ => (modules.iter().map(String::as_str).collect(), segments),
        };

        // 2. Each `self` stays, and each `super` leaves the module.
        while let Some((&step @ ("self" | "super"), tail)) = rest.split_first() {
            if step == "super" && modules.pop().is_none() {
                return Resolved::failed(message!(rust_super_above_root));
            }
            rest = tail;
        }

        modules.extend(rest);
        self.module(target, &modules, path)
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
                let krate = self.crates.get(*index);
                return match krate.library() {
                    Some(library) => self.module(library, rest, path),
                    None => Resolved::failed(message!(rust_no_library, name = krate.name)),
                };
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
            return Resolved::ignored(message!(rust_type_in_scope));
        }
        if !rooted && let Some(Position { target, modules }) = krate.locate(path) {
            let mut all: Vec<&str> = modules.iter().map(String::as_str).collect();
            let own = all.len();
            all.push(name);
            all.extend(rest);
            let (file, depth) = target.module_file(&all);
            if depth > own {
                return self.file(&file, path);
            }
        }
        // 3. A tool's attribute or lint, which only names a crate or module when one
        //    has its name.
        if !rooted && TOOLS.contains(&name) {
            return Resolved::ignored(message!(rust_tool, name));
        }
        Resolved::failed(message!(rust_not_a_crate, name))
    }

    /// The file of the module that `modules` name in the tree of `target`,
    /// for an import in the file at `importer`.
    fn module(&self, target: &Target, modules: &[&str], importer: &Path) -> Resolved {
        self.file(&target.module_file(modules).0, importer)
    }

    /// `file`, imported by the file at `importer`, as it is named in the
    /// project. A file does not import itself.
    fn file(&self, file: &Path, importer: &Path) -> Resolved {
        if file == importer {
            return Resolved::ignored(message!(rust_importing_file));
        }
        match relative_posix(self.root, file) {
            Some(name) => Resolved::Internal(name),
            None => Resolved::unresolved(UnresolvedReason::OutsideRoot(file.to_path_buf())),
        }
    }
}
