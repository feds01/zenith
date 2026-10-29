//! The file system that the resolver sees, with the workspace packages linked
//! into the project's `node_modules`.
//!
//! A package manager links each workspace package into the root
//! `node_modules`, and resolution relies on those links: Node's lookup of a
//! bare specifier, and `oxc_resolver`'s of a `tsconfig.json` that `extends`
//! one from a package. Without them, `extends` fails, and lookup climbs past
//! the root into the `node_modules` of whatever directory holds the project,
//! where a git worktree finds the links of the main checkout, and so packages
//! outside the project.
//!
//! So wherever nothing is installed, [`LinkedFileSystem`] shows
//! `<root>/node_modules/<name>` as a symlink to the workspace package of that
//! name, with the directories that hold it. What is installed always wins.
//!
//! It shows the project as the session's file system does, which `oxc_resolver`
//! reads through its own [`FileSystem`] trait.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use oxc_resolver::{FileMetadata, FileSystem, FileSystemOs, ResolveError};
use rustc_hash::{FxHashMap, FxHashSet};
use zenith_utils::{
    fs::{self, FileKind, disk},
    path::{absolute_path, relative_posix},
    specifier::package_name,
};

/// The metadata of a directory.
const DIR: FileMetadata = FileMetadata::new(false, true, false);

/// The metadata of a symlink.
const SYMLINK: FileMetadata = FileMetadata::new(false, false, true);

/// The session's file system, with the workspace packages linked into
/// `<root>/node_modules` where nothing is installed.
#[derive(Debug)]
pub(crate) struct LinkedFileSystem {
    /// The session's file system.
    file_system: Arc<dyn fs::FileSystem>,

    /// The `node_modules` directory in the project root.
    node_modules: PathBuf,

    /// The directory of each workspace package, by name.
    packages: FxHashMap<String, PathBuf>,

    /// The scopes of the scoped packages, such as `@acme`.
    scopes: FxHashSet<String>,
}

/// What the links put at a path.
#[derive(Debug, PartialEq, Eq)]
enum Linked<'a> {
    /// `node_modules`, or the directory of a scope in it.
    Dir,

    /// The link to a package, which points at this directory.
    Link(&'a Path),

    /// A path inside a link, which is really this path.
    Inside(PathBuf),
}

impl LinkedFileSystem {
    /// Link `packages`, by name, with their absolute directories, into
    /// `node_modules` in `root`, which is canonical in `file_system`.
    pub(crate) fn new(
        root: &Path,
        file_system: Arc<dyn fs::FileSystem>,
        packages: FxHashMap<String, PathBuf>,
    ) -> Self {
        let scopes = packages
            .keys()
            .filter_map(|name| name.split_once('/'))
            .filter(|(scope, _)| scope.starts_with('@'))
            .map(|(scope, _)| scope.to_string())
            .collect();
        let node_modules = root.join("node_modules");
        Self { file_system, node_modules, packages, scopes }
    }

    /// What the links show at `path`, if anything. Something installed there
    /// always wins.
    fn linked(&self, path: &Path) -> Option<Linked<'_>> {
        let linked = self.link_at(path)?;
        let missing = (self.file_system.link_kind(path))
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound);
        missing.then_some(linked)
    }

    /// The metadata of `path` in the session's file system, following links
    /// if `follow` is set.
    fn metadata_of(&self, path: &Path, follow: bool) -> io::Result<FileMetadata> {
        let kind = match follow {
            true => self.file_system.kind(path)?,
            false => self.file_system.link_kind(path)?,
        };
        Ok(match kind {
            FileKind::File => FileMetadata::new(true, false, false),
            FileKind::Directory => DIR,
            FileKind::Symlink => SYMLINK,
            FileKind::Other => FileMetadata::new(false, false, false),
        })
    }

    /// What the links put at `path`, whatever is installed there.
    fn link_at(&self, path: &Path) -> Option<Linked<'_>> {
        // A path whose text doesn't name `node_modules` can't be in it, which is
        // quicker to tell than its path relative to it.
        if path.to_str().is_some_and(|path| !path.contains("node_modules")) {
            return None;
        }

        // `@acme/ui/src/index.ts`, or `""` for `node_modules` itself.
        let relative = relative_posix(&self.node_modules, path)?;
        if relative.is_empty() || self.scopes.contains(&relative) {
            return Some(Linked::Dir);
        }
        let name = package_name(&relative)?;
        let dir = self.packages.get(name)?;
        match relative[name.len()..].strip_prefix('/') {
            None => Some(Linked::Link(dir)),
            Some(inside) => Some(Linked::Inside(absolute_path(dir, inside))),
        }
    }
}

impl FileSystem for LinkedFileSystem {
    /// The disk, with no packages linked, since there is no root to link
    /// them into.
    fn new() -> Self {
        Self::new(Path::new(""), disk().clone(), FxHashMap::default())
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.linked(path) {
            None => self.file_system.read(path),
            Some(Linked::Inside(real)) => self.file_system.read(&real),
            Some(Linked::Dir | Linked::Link(_)) => Err(io::ErrorKind::IsADirectory.into()),
        }
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        FileSystemOs::validate_string(self.read(path)?)
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        match self.linked(path) {
            None => self.metadata_of(path, true),
            Some(Linked::Dir) => Ok(DIR),
            Some(Linked::Link(dir)) => self.metadata_of(dir, true),
            Some(Linked::Inside(real)) => self.metadata_of(&real, true),
        }
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        match self.linked(path) {
            None => self.metadata_of(path, false),
            Some(Linked::Dir) => Ok(DIR),
            Some(Linked::Link(_)) => Ok(SYMLINK),
            Some(Linked::Inside(real)) => self.metadata_of(&real, false),
        }
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        match self.linked(path) {
            None => Ok(self.file_system.read_link(path)?),
            Some(Linked::Dir) => Err(io::Error::from(io::ErrorKind::InvalidInput).into()),
            Some(Linked::Link(dir)) => Ok(dir.to_path_buf()),
            Some(Linked::Inside(real)) => Ok(self.file_system.read_link(&real)?),
        }
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        match self.linked(path) {
            None => self.file_system.canonicalize(path),
            Some(Linked::Dir) => Ok(path.to_path_buf()),
            Some(Linked::Link(dir)) => self.file_system.canonicalize(dir),
            Some(Linked::Inside(real)) => self.file_system.canonicalize(&real),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn file_system(root: &Path) -> LinkedFileSystem {
        let packages = [
            ("@acme/ui".to_string(), root.join("packages/ui")),
            ("tsconfig".to_string(), root.join("packages/tsconfig")),
        ];
        LinkedFileSystem::new(root, disk().clone(), packages.into_iter().collect())
    }

    #[test]
    fn links_packages_into_the_root_node_modules() {
        let root = Path::new("/repo");
        let file_system = file_system(root);
        let link_at = |path: &str| file_system.link_at(&root.join(path));
        let ui = root.join("packages/ui");

        assert_eq!(link_at("node_modules"), Some(Linked::Dir));
        assert_eq!(link_at("node_modules/@acme"), Some(Linked::Dir));
        assert_eq!(link_at("node_modules/@acme/ui"), Some(Linked::Link(&ui)));
        assert_eq!(
            link_at("node_modules/@acme/ui/src/index.ts"),
            Some(Linked::Inside(ui.join("src/index.ts")))
        );
        assert_eq!(
            link_at("node_modules/tsconfig/base.json"),
            Some(Linked::Inside(root.join("packages/tsconfig/base.json")))
        );
    }

    #[test]
    fn links_nothing_else() {
        let root = Path::new("/repo");
        let file_system = file_system(root);
        for path in [
            "node_modules/@vendor",
            "node_modules/@acme/sdk",
            "node_modules/@acme/uikit",
            "node_modules/react",
            "packages/ui/node_modules/@acme/ui",
        ] {
            assert_eq!(file_system.link_at(&root.join(path)), None, "{path}");
        }
        assert_eq!(file_system.link_at(Path::new("/node_modules/@acme/ui")), None);
    }

    #[test]
    fn what_is_installed_wins() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("node_modules/@acme/ui")).unwrap();
        let file_system = file_system(root);

        assert_eq!(file_system.linked(&root.join("node_modules")), None);
        assert_eq!(file_system.linked(&root.join("node_modules/@acme/ui")), None);
        assert_eq!(
            file_system.linked(&root.join("node_modules/tsconfig")),
            Some(Linked::Link(&root.join("packages/tsconfig")))
        );
    }
}
