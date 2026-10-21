//! The crates of a project: found from its `Cargo.toml` files, each with the
//! crates it depends on.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use log::warn;
use rustc_hash::FxHashMap;
use zenith_lang::{Workspace, WorkspacePackage};
use zenith_utils::{
    fs::read_text,
    path::{normalize_path, relative_posix},
};

use crate::manifest::{Dependency as Declared, Manifest};

/// The crates of the standard library, which every crate may use.
pub(crate) const STANDARD_CRATES: &[&str] = &["std", "core", "alloc", "proc_macro", "test"];

/// The crates of a project.
#[derive(Debug, Default)]
pub(crate) struct Crates {
    /// Every crate, sorted by directory.
    crates: Vec<Crate>,

    /// The index in [`crates`](Self::crates) of the crate in each directory.
    by_dir: FxHashMap<PathBuf, usize>,
}

/// A crate of the project: a Cargo package.
#[derive(Debug)]
pub(crate) struct Crate {
    /// The name of the package.
    pub(crate) name: String,

    /// Its directory, as an absolute path.
    pub(crate) dir: PathBuf,

    /// The name that code uses for its library.
    pub(crate) lib_name: String,

    /// The root of its library, as a POSIX path relative to its directory.
    pub(crate) lib_path: String,

    /// The crates that it uses, by the name that its code uses for each.
    pub(crate) dependencies: FxHashMap<String, Dependency>,
}

/// A crate that another crate depends on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Dependency {
    /// A crate of the project, by its index in [`Crates`].
    Internal(usize),

    /// A crate from outside the project, by its package name.
    External(String),
}

impl Crates {
    /// The crates of a project, which are the packages of the `manifests`
    /// that the walk found.
    pub(crate) fn discover(manifests: &[PathBuf]) -> Self {
        // 1. Read the manifests.
        let manifests: Vec<(&Path, Manifest)> = manifests
            .iter()
            .filter_map(|path| {
                let manifest =
                    read_text(path).map_err(|error| error.to_string()).and_then(|text| {
                        Manifest::parse(&text).map_err(|error| error.message().to_string())
                    });
                match manifest {
                    Ok(manifest) => Some((path.parent()?, manifest)),
                    Err(error) => {
                        warn!("could not read `{}`: {error}", path.display());
                        None
                    }
                }
            })
            .collect();

        // 2. Every manifest with a package is a crate.
        let mut members: Vec<_> =
            manifests.iter().filter(|(_, manifest)| manifest.package.is_some()).collect();
        members.sort_by_key(|(dir, _)| *dir);
        let crates: Vec<_> =
            members.iter().map(|(dir, manifest)| Crate::new(dir, manifest)).collect();
        let by_dir = (0..).zip(&crates).map(|(index, krate)| (krate.dir.clone(), index)).collect();
        let mut crates = Self { crates, by_dir };

        // 3. Find what each one depends on, now that every crate is known.
        let workspaces: FxHashMap<&Path, &Manifest> = manifests
            .iter()
            .filter(|(_, manifest)| manifest.workspace.is_some())
            .map(|(dir, manifest)| (*dir, manifest))
            .collect();
        let dependencies: Vec<_> = members
            .iter()
            .map(|(dir, manifest)| {
                let workspace = dir.ancestors().find_map(|dir| Some((dir, *workspaces.get(dir)?)));
                crates.dependencies(dir, manifest, workspace)
            })
            .collect();
        for (index, (krate, mut dependencies)) in
            crates.crates.iter_mut().zip(dependencies).enumerate()
        {
            // A crate is known to its own tests and examples by its name.
            dependencies.entry(krate.lib_name.clone()).or_insert(Dependency::Internal(index));
            krate.dependencies = dependencies;
        }
        crates
    }

    /// The dependencies of the crate in `dir`, whose manifest is `manifest`,
    /// as its code names them. `workspace` is the directory and manifest of
    /// the workspace that the crate is in, which declares the dependencies
    /// that `workspace = true` takes.
    fn dependencies(
        &self,
        dir: &Path,
        manifest: &Manifest,
        workspace: Option<(&Path, &Manifest)>,
    ) -> FxHashMap<String, Dependency> {
        manifest
            .dependencies()
            .map(|(name, declared)| {
                // 1. A dependency of the workspace is declared in its manifest, where paths are
                //    relative to it.
                let shared = workspace.and_then(|(workspace_dir, manifest)| {
                    let shared = manifest.workspace.as_ref()?.dependencies.get(name)?;
                    Some((workspace_dir, shared))
                });
                let (base, declared) = match declared {
                    Declared::Table(table) if table.workspace => shared.unwrap_or((dir, declared)),
                    _ => (dir, declared),
                };

                // 2. A path to a crate of the project is that crate. Any other is external.
                let (path, package) = match declared {
                    Declared::Version(_) => (None, None),
                    Declared::Table(table) => (table.path.as_deref(), table.package.as_deref()),
                };
                let internal =
                    path.and_then(|path| self.by_dir.get(&normalize_path(base.join(path))));
                let dependency = match internal {
                    Some(&index) => Dependency::Internal(index),
                    None => Dependency::External(package.unwrap_or(name).to_owned()),
                };
                (name.replace('-', "_"), dependency)
            })
            .collect()
    }

    /// The crate at `index`.
    pub(crate) fn get(&self, index: usize) -> &Crate {
        &self.crates[index]
    }

    /// The nearest crate whose directory contains `path`.
    pub(crate) fn containing(&self, path: &Path) -> Option<&Crate> {
        path.ancestors().find_map(|dir| self.by_dir.get(dir)).map(|&index| self.get(index))
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

impl Crate {
    /// The crate of the package in `dir`, which has a `[package]`.
    fn new(dir: &Path, manifest: &Manifest) -> Self {
        let name =
            manifest.package.as_ref().map(|package| package.name.clone()).unwrap_or_default();
        let lib = manifest.lib.as_ref();
        Self {
            lib_name: (lib.and_then(|lib| lib.name.clone()))
                .unwrap_or_else(|| name.replace('-', "_")),
            lib_path: (lib.and_then(|lib| lib.path.clone())).unwrap_or_else(|| "src/lib.rs".into()),
            name,
            dir: dir.to_path_buf(),
            dependencies: FxHashMap::default(),
        }
    }
}
