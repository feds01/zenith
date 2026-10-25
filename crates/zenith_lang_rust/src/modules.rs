//! The module tree of a crate: where each file sits in it, which file each
//! module is, and which files only tests compile.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use rustc_hash::FxHashSet;
use zenith_graph::ImportKind;
use zenith_lang::ParsedFile;
use zenith_utils::fs::FileSystem;

use crate::crates::{Crate, Crates, Target, TargetKind};

/// Where a file sits in the module tree of its crate.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Position<'c> {
    /// The target whose tree it is in.
    pub(crate) target: &'c Target,

    /// The names of the modules from the root to the file's module, which are
    /// none for the root itself.
    pub(crate) modules: Vec<String>,
}

impl Target {
    /// The directory that the files of the submodules of the root are in,
    /// which is that of the root.
    fn dir(&self) -> &Path {
        self.root.parent().unwrap_or(&self.root)
    }

    /// The file of the module that `modules`, the names of the modules from
    /// the root, name in `file_system`, and how many of them it takes to name
    /// it. A module is in `a.rs` or `a/mod.rs`, in the directory of its
    /// parent, spelt as the module is even where names ignore case, so that
    /// the type `Config` is not the module `config`. Only the longest prefix
    /// of `modules` that is a file counts, as the rest are items in it, or
    /// modules that it declares inline. With no such prefix, the module is
    /// the root.
    pub(crate) fn module_file(
        &self,
        file_system: &dyn FileSystem,
        modules: &[&str],
    ) -> (PathBuf, usize) {
        let mut dir = self.dir().to_path_buf();
        let mut found = (self.root.clone(), 0);
        for (depth, module) in modules.iter().enumerate() {
            dir.push(module);
            // Each file, with the entry that is named after the module.
            let files = [
                (dir.with_extension("rs"), dir.with_extension("rs")),
                (dir.join("mod.rs"), dir.clone()),
            ];
            let Some((file, _)) = files
                .into_iter()
                .find(|(file, named)| file_system.is_file(file) && file_system.is_listed(named))
            else {
                break;
            };
            found = (file, depth + 1);
        }
        found
    }
}

impl Crate {
    /// The library of the crate.
    pub(crate) fn library(&self) -> Option<&Target> {
        self.targets.iter().find(|target| target.kind == TargetKind::Lib)
    }

    /// Where the file at `path` sits in the module tree of the crate. `None`
    /// if it is in none. A root file is the root of the tree of its target.
    /// Any other file is in the tree of the target whose directory is the
    /// nearest above it, or the first of those that share it, which is the
    /// library if it is one of them. Build scripts have no tree but their own.
    pub(crate) fn locate(&self, path: &Path) -> Option<Position<'_>> {
        if let Some(target) = self.targets.iter().find(|target| target.root == path) {
            return Some(Position { target, modules: Vec::new() });
        }
        let nearest = (self.targets.iter())
            .filter(|target| target.kind != TargetKind::Build && path.starts_with(target.dir()))
            .reduce(|best, next| {
                match next.dir().as_os_str().len() > best.dir().as_os_str().len() {
                    true => next,
                    false => best,
                }
            })?;
        let relative = path.strip_prefix(nearest.dir()).ok()?;
        let mut modules: Vec<String> = (relative.with_extension("").components())
            .map(|module| module.as_os_str().to_string_lossy().into_owned())
            .collect();
        if path.extension()? != "rs" {
            return None;
        }
        // `a/mod.rs` is the module `a`.
        if modules.last().is_some_and(|module| module == "mod") {
            modules.pop();
        }
        Some(Position { target: nearest, modules })
    }
}

impl Crates {
    /// Make every import of each of `files`, each with its path, that only
    /// tests compile a test import: the files of the tests, benchmarks and
    /// examples, and those of the modules that a file declares for its tests
    /// alone, as `#[cfg(test)] mod tests;` does, with every module in them.
    pub(crate) fn mark_test_code(&self, files: &mut [(PathBuf, &mut ParsedFile)]) {
        // 1. Where each file sits, and the modules that the files declare for their
        //    tests, by the root of their target and their path from it.
        let positions: Vec<_> =
            files.iter().map(|(path, _)| self.containing(path)?.locate(path)).collect();
        let mut declared: FxHashSet<(&Path, String)> = FxHashSet::default();
        for ((_, parsed), position) in files.iter().zip(&positions) {
            let Some(Position { target, modules }) = position else {
                continue;
            };
            for module in &parsed.test_modules {
                let path = modules.iter().map(String::as_str).chain(module.split("::"));
                declared.insert((&target.root, path.collect::<Vec<_>>().join("::")));
            }
        }

        // 2. A file is test code if its target is, or a module on its path is.
        for ((_, parsed), position) in files.iter_mut().zip(&positions) {
            let Some(Position { target, modules }) = position else {
                continue;
            };
            let mut path = String::new();
            let in_test_module = modules.iter().any(|module| {
                if !path.is_empty() {
                    path.push_str("::");
                }
                path.push_str(module);
                declared.contains(&(target.root.as_path(), path.clone()))
            });
            let test_target =
                matches!(target.kind, TargetKind::Test | TargetKind::Bench | TargetKind::Example);
            if test_target || in_test_module {
                for import in &mut parsed.imports {
                    import.kind = ImportKind::Test;
                }
                for import in &mut parsed.non_literal_imports {
                    import.value = ImportKind::Test;
                }
            }
        }
    }
}
