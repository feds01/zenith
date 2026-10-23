//! Where the modules of a project are: the roots that absolute imports start
//! from, as `sys.path` lists them, and the file of each module in a
//! directory.

#[cfg(test)]
mod tests;

use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock},
};

use log::warn;
use rustc_hash::{FxHashMap, FxHashSet};
use zenith_utils::path::normalize_path;

use crate::packages::Package;

/// The extensions of the files of a module, the source before the stub.
const EXTENSIONS: [&str; 2] = ["py", "pyi"];

/// The roots that absolute imports start from.
#[derive(Debug, Default)]
pub(crate) struct Roots {
    /// The roots, as absolute paths, in the order that they are searched.
    dirs: Vec<PathBuf>,

    /// The indices in [`dirs`](Self::dirs) of the roots that have a module
    /// or a directory of each name, in order.
    by_name: FxHashMap<String, Vec<usize>>,
}

impl Roots {
    /// The roots of the project at `root`, with its `packages`: the root of
    /// each package, then the project root.
    pub(crate) fn inferred(root: &Path, packages: &[Package]) -> Self {
        let dirs = packages.iter().map(Package::root).chain([root.to_path_buf()]);
        Self::new(dirs.collect())
    }

    /// The roots that the configuration names, each relative to the project
    /// at `root`. Those that aren't directories are left out, with a warning.
    pub(crate) fn configured(root: &Path, names: &[String]) -> Self {
        let dirs = names
            .iter()
            .filter_map(|name| {
                let dir = normalize_path(root.join(name));
                match dir.is_dir() {
                    true => Some(dir),
                    false => {
                        warn!("the Python root `{name}` is not a directory");
                        None
                    }
                }
            })
            .collect();
        Self::new(dirs)
    }

    /// The roots `dirs`, in the order that they are searched. A root that is
    /// listed again keeps its first place.
    fn new(mut dirs: Vec<PathBuf>) -> Self {
        let mut seen = Vec::new();
        dirs.retain(|dir| match seen.contains(dir) {
            true => false,
            false => {
                seen.push(dir.clone());
                true
            }
        });
        let mut by_name: FxHashMap<String, Vec<usize>> = FxHashMap::default();
        for (index, dir) in dirs.iter().enumerate() {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = match path.is_dir() {
                    true => path.file_name(),
                    false if is_module_file(&path) => path.file_stem(),
                    false => None,
                };
                if let Some(name) = name.and_then(|name| name.to_str()) {
                    let indices = by_name.entry(name.to_owned()).or_default();
                    if indices.last() != Some(&index) {
                        indices.push(index);
                    }
                }
            }
        }
        Self { dirs, by_name }
    }

    /// The roots that have a module or a directory named `name`: those that
    /// contain the file at `importer` first, the nearest first, then the
    /// others in order.
    pub(crate) fn having(&self, name: &str, importer: &Path) -> Vec<&Path> {
        let Some(indices) = self.by_name.get(name) else {
            return Vec::new();
        };
        let mut roots: Vec<&Path> =
            indices.iter().map(|&index| self.dirs[index].as_path()).collect();
        // The sort is stable, so the roots that don't contain the file stay in order.
        roots.sort_by_key(|dir| match importer.starts_with(dir) {
            true => usize::MAX - dir.as_os_str().len(),
            false => usize::MAX,
        });
        roots
    }
}

/// What the directories of a project hold, each read once, as finding
/// modules needs it. Names are compared exactly, as Python's importer
/// compares them, even where the file system ignores case.
#[derive(Debug, Default)]
pub(crate) struct Directories {
    /// What each directory that was read holds.
    listings: RwLock<FxHashMap<PathBuf, Arc<Listing>>>,
}

/// What one directory holds. One that can't be read holds nothing.
#[derive(Debug, Default)]
struct Listing {
    /// The names of its files.
    files: FxHashSet<OsString>,

    /// The names of its directories.
    dirs: FxHashSet<OsString>,
}

impl Directories {
    /// What `dir` holds, read the first time that it is asked for.
    fn listing(&self, dir: &Path) -> Arc<Listing> {
        let listings = self.listings.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(listing) = listings.get(dir) {
            return Arc::clone(listing);
        }
        drop(listings);
        let mut listing = Listing::default();
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            match entry.path().is_dir() {
                true => listing.dirs.insert(entry.file_name()),
                false => listing.files.insert(entry.file_name()),
            };
        }
        let listing = Arc::new(listing);
        let mut listings = self.listings.write().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(listings.entry(dir.to_path_buf()).or_insert(listing))
    }

    /// The file named `name` in `dir`, if there is one.
    fn file(&self, dir: &Path, name: &str) -> Option<PathBuf> {
        self.listing(dir).files.contains(OsStr::new(name)).then(|| dir.join(name))
    }

    /// The file of the module `name` in `dir`, if there is one: its source,
    /// or else its stub.
    fn module_file(&self, dir: &Path, name: &str) -> Option<PathBuf> {
        EXTENSIONS.iter().find_map(|ext| self.file(dir, &format!("{name}.{ext}")))
    }

    /// The `__init__.py` of `dir`, or its `__init__.pyi`, if it is a regular
    /// package.
    pub(crate) fn init_file(&self, dir: &Path) -> Option<PathBuf> {
        self.module_file(dir, "__init__")
    }

    /// The directory that holds the outermost regular package of the file at
    /// `path`, which is where its absolute imports start when it is run from
    /// there: the nearest directory above it without an `__init__.py`, but
    /// never one above `root`.
    pub(crate) fn package_root<'p>(&self, path: &'p Path, root: &Path) -> &'p Path {
        let mut dir = path.parent().unwrap_or(path);
        while dir != root
            && self.init_file(dir).is_some()
            && let Some(parent) = dir.parent()
        {
            dir = parent;
        }
        dir
    }
}

/// A module that a path of names leads to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Module {
    /// Its file, as an absolute path.
    pub(crate) file: PathBuf,

    /// Whether the first name is a regular package or a module, rather than a
    /// namespace package, a directory without an `__init__.py`.
    pub(crate) regular: bool,
}

impl Module {
    /// The module that `names` lead to from `dir`, among `directories`: the
    /// last of them that is a module or a regular package, as the rest are
    /// names in it. A name is a package if it is a directory with an
    /// `__init__.py`, or else a module if it is a `.py` file, or else a
    /// namespace package if it is a directory, which leads on but has no
    /// file. `None` if no module is found, which a path of namespace packages
    /// alone isn't.
    pub(crate) fn find(directories: &Directories, dir: &Path, names: &[&str]) -> Option<Module> {
        let mut dir = dir.to_path_buf();
        let mut found = None;
        let mut regular = false;
        for (depth, name) in names.iter().enumerate() {
            let is_dir = directories.listing(&dir).dirs.contains(OsStr::new(name));
            let package = is_dir.then(|| dir.join(name));
            if let Some(init) =
                package.as_deref().and_then(|package| directories.init_file(package))
            {
                regular |= depth == 0;
                found = Some(init);
            } else if let Some(file) = directories.module_file(&dir, name) {
                regular |= depth == 0;
                found = Some(file);
                break;
            }
            match package {
                Some(package) => dir = package,
                None => break,
            }
        }
        found.map(|file| Module { file, regular })
    }
}

/// Whether `path` is the file of a module, by its extension.
fn is_module_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| EXTENSIONS.iter().any(|known| ext == *known))
}
