//! The crates of a project, each with its targets and the crates it depends
//! on. Cargo can say what they are, and failing that, the `Cargo.toml` files
//! can.

mod cargo;
pub(crate) mod manifest;
mod manifests;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use manifest::Manifest;
use ra_ap_syntax::Edition;
use rustc_hash::FxHashMap;
use zenith_lang::{Workspace, WorkspacePackage};
use zenith_utils::{fs::FileSystem, path::relative_posix};

/// The crates of the standard library, which every crate may use.
pub(crate) const STANDARD_CRATES: &[&str] = &["std", "core", "alloc", "proc_macro", "test"];

/// The crates of a project.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Crates {
    /// Every crate, sorted by directory.
    crates: Vec<Crate>,

    /// The index in [`crates`](Self::crates) of the crate in each directory.
    by_dir: FxHashMap<PathBuf, usize>,
}

/// A crate of the project: a Cargo package.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Crate {
    /// The name of the package.
    pub(crate) name: String,

    /// Its directory, as an absolute path.
    pub(crate) dir: PathBuf,

    /// The edition of Rust that its code is written in.
    pub(crate) edition: Edition,

    /// Its targets, sorted by kind, then by root.
    pub(crate) targets: Vec<Target>,

    /// The crates that it uses, by the name that its code uses for each.
    pub(crate) dependencies: FxHashMap<String, Dependency>,
}

/// A target of a crate: a library, a binary, a test, and so on, which is the
/// root of a tree of modules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Target {
    /// What the target is.
    pub(crate) kind: TargetKind,

    /// The file of its root module, as an absolute path.
    pub(crate) root: PathBuf,
}

/// What a [`Target`] is. They are ordered so that the library comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TargetKind {
    /// The library, which the other targets and other crates use.
    Lib,

    /// A binary.
    Bin,

    /// An integration test.
    Test,

    /// A benchmark.
    Bench,

    /// An example.
    Example,

    /// The build script.
    Build,
}

/// A crate that another crate depends on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Dependency {
    /// A crate of the project, by its index in [`Crates`].
    Internal(usize),

    /// A crate from outside the project, by its package name.
    External(String),
}

/// A package, as Cargo or its manifest says, before its dependencies are
/// told apart as those of the project and those from outside.
pub(crate) struct Found {
    /// The name of the package.
    pub(crate) name: String,

    /// Its directory, as an absolute path.
    pub(crate) dir: PathBuf,

    /// The edition of Rust that its code is written in.
    pub(crate) edition: Edition,

    /// The name that code uses for its library, if it has one.
    pub(crate) library: Option<String>,

    /// Its targets.
    pub(crate) targets: Vec<Target>,

    /// Its dependencies.
    pub(crate) dependencies: Vec<Declared>,
}

/// A dependency of a package, as declared.
pub(crate) struct Declared {
    /// The name that the code of the package uses for it.
    pub(crate) name: String,

    /// The name of the package that it is.
    pub(crate) package: String,

    /// Its directory, if it is a path, as an absolute path.
    pub(crate) path: Option<PathBuf>,
}

impl Crates {
    /// The crates of the project at `root` in `file_system`, which must be
    /// canonical there, in the packages of the `manifests` that the walk
    /// found. Cargo says what they are, or, if it can't, or the project is
    /// not on the disk, where Cargo would read it, the manifests do.
    pub(crate) fn discover(
        file_system: &dyn FileSystem,
        root: &Path,
        manifests: &[PathBuf],
    ) -> Self {
        let manifests = Manifest::read_all(file_system, manifests);
        if !file_system.on_disk() {
            return Self::from_manifests(file_system, &manifests);
        }
        Self::from_cargo(root, &manifests).unwrap_or_else(|error| {
            error.log();
            Self::from_manifests(file_system, &manifests)
        })
    }

    /// The crates of `found`, which may be in any order.
    fn new(mut found: Vec<Found>) -> Self {
        found.sort_by(|a, b| a.dir.cmp(&b.dir));
        let by_dir: FxHashMap<_, _> =
            found.iter().enumerate().map(|(index, found)| (found.dir.clone(), index)).collect();
        let crates = found
            .into_iter()
            .enumerate()
            .map(|(index, found)| {
                // A dependency is internal if its path is the directory of a crate.
                let mut dependencies: FxHashMap<_, _> = (found.dependencies.into_iter())
                    .map(|declared| {
                        let internal = declared.path.and_then(|path| by_dir.get(&path).copied());
                        (
                            declared.name,
                            match internal {
                                Some(index) => Dependency::Internal(index),
                                None => Dependency::External(declared.package),
                            },
                        )
                    })
                    .collect();
                // The other targets of a crate use its library by its name.
                if let Some(library) = found.library {
                    dependencies.entry(library).or_insert(Dependency::Internal(index));
                }
                let mut targets = found.targets;
                targets.sort();
                targets.dedup();
                let (name, dir, edition) = (found.name, found.dir, found.edition);
                Crate { name, dir, edition, targets, dependencies }
            })
            .collect();
        Self { crates, by_dir }
    }

    /// The crate at `index`.
    pub(crate) fn get(&self, index: usize) -> &Crate {
        &self.crates[index]
    }

    /// The nearest crate whose directory contains `path`.
    pub(crate) fn containing(&self, path: &Path) -> Option<&Crate> {
        path.ancestors().find_map(|dir| self.by_dir.get(dir)).map(|&index| self.get(index))
    }

    /// The edition of each crate, by its directory.
    pub(crate) fn editions(&self) -> Editions {
        Editions(self.crates.iter().map(|krate| (krate.dir.clone(), krate.edition)).collect())
    }

    /// The crates as the workspace packages of the project at `root`, which
    /// is the root package if a crate is in it.
    pub(crate) fn workspace(&self, root: &Path) -> Workspace {
        let mut workspace = Workspace::default();
        for krate in &self.crates {
            let Some(dir) = relative_posix(root, &krate.dir) else {
                continue;
            };
            let package =
                WorkspacePackage { name: krate.name.clone(), dir, path: krate.dir.clone() };
            match package.dir.is_empty() {
                true => workspace.root_package = Some(package),
                false => workspace.packages.push(package),
            }
        }
        workspace.packages.sort_by(|a, b| a.dir.cmp(&b.dir));
        workspace
    }
}

/// The edition of each crate of a project, by its directory, which the
/// parsers of every thread share.
#[derive(Debug, Default)]
pub(crate) struct Editions(pub(crate) FxHashMap<PathBuf, Edition>);

impl Editions {
    /// The edition of the code of the file at `path`: that of the nearest
    /// crate whose directory contains it, or the latest for a file in none.
    pub(crate) fn of(&self, path: &Path) -> Edition {
        path.ancestors().find_map(|dir| self.0.get(dir).copied()).unwrap_or(Edition::LATEST)
    }
}

/// The edition that a manifest or Cargo names, as `"2021"`. Cargo's default,
/// 2015, if it names none, and the latest if it names one that is newer.
pub(crate) fn edition(name: Option<&str>) -> Edition {
    match name {
        None => Edition::DEFAULT,
        Some(name) => name.parse().unwrap_or(Edition::LATEST),
    }
}
