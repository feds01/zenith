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

use std::{
    borrow::Cow,
    fs, io,
    path::{Path, PathBuf},
};

use oxc_resolver::{FileMetadata, FileSystem, FileSystemOs, ResolveError};
use rustc_hash::{FxHashMap, FxHashSet};

/// The operating system's file system, with the workspace packages linked
/// into `<root>/node_modules` where nothing is installed.
#[derive(Debug)]
pub(crate) struct LinkedFileSystem {
    /// The operating system's file system.
    os: FileSystemOs,

    /// The `node_modules` directory in the project root.
    node_modules: PathBuf,

    /// The directory of each workspace package, by name.
    packages: FxHashMap<String, PathBuf>,

    /// The scopes of the scoped packages, such as `@acme`.
    scopes: FxHashSet<String>,
}

/// What the links add at a path where nothing exists.
#[derive(Debug, PartialEq, Eq)]
enum Entry<'a> {
    /// `node_modules`, or a scope's directory in it.
    Dir,

    /// The link to a package, and the package's directory.
    Link(&'a Path),

    /// A path inside a link, and where it really is.
    Inside(PathBuf),
}

impl LinkedFileSystem {
    /// Link `packages`, by name, with their absolute directories, into
    /// `node_modules` in the canonical `root`.
    pub(crate) fn new(root: &Path, packages: FxHashMap<String, PathBuf>) -> Self {
        let scopes = packages
            .keys()
            .filter_map(|name| name.split_once('/'))
            .filter(|(scope, _)| scope.starts_with('@'))
            .map(|(scope, _)| scope.to_string())
            .collect();
        Self { os: FileSystemOs::new(), node_modules: root.join("node_modules"), packages, scopes }
    }

    /// What the links add at `path`, if anything. Whether something is
    /// installed there is up to the caller.
    fn entry(&self, path: &Path) -> Option<Entry<'_>> {
        let mut components = path.strip_prefix(&self.node_modules).ok()?.components();
        let Some(first) = components.next() else { return Some(Entry::Dir) };
        let first = first.as_os_str().to_str()?;
        let name = if first.starts_with('@') {
            match components.next() {
                Some(second) => Cow::Owned(format!("{first}/{}", second.as_os_str().to_str()?)),
                None => return self.scopes.contains(first).then_some(Entry::Dir),
            }
        } else {
            Cow::Borrowed(first)
        };
        let dir = self.packages.get(name.as_ref())?;
        match components.as_path() {
            rest if rest.as_os_str().is_empty() => Some(Entry::Link(dir)),
            rest => Some(Entry::Inside(dir.join(rest))),
        }
    }

    /// What the links add at `path`, given that the operating system failed
    /// with `error` there. Unless nothing exists there, `error` stands.
    fn missing(&self, path: &Path, error: io::Error) -> io::Result<Entry<'_>> {
        // On Windows, `oxc_resolver`'s errors don't say that a path is
        // missing, so ask the standard library.
        match self.entry(path) {
            Some(entry)
                if fs::symlink_metadata(path)
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound) =>
            {
                Ok(entry)
            }
            _ => Err(error),
        }
    }
}

impl FileSystem for LinkedFileSystem {
    /// No packages are linked, since there is no root to link them into.
    fn new() -> Self {
        Self::new(Path::new(""), FxHashMap::default())
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.os.read(path).or_else(|error| match self.missing(path, error)? {
            Entry::Inside(real) => self.os.read(&real),
            Entry::Dir | Entry::Link(_) => Err(io::ErrorKind::IsADirectory.into()),
        })
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        FileSystemOs::validate_string(self.read(path)?)
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.os.metadata(path).or_else(|error| match self.missing(path, error)? {
            Entry::Dir => Ok(FileMetadata::new(false, true, false)),
            Entry::Link(real) => self.os.metadata(real),
            Entry::Inside(real) => self.os.metadata(&real),
        })
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.os.symlink_metadata(path).or_else(|error| match self.missing(path, error)? {
            Entry::Dir => Ok(FileMetadata::new(false, true, false)),
            Entry::Link(_) => Ok(FileMetadata::new(false, false, true)),
            Entry::Inside(real) => self.os.symlink_metadata(&real),
        })
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        let entry = match self.os.symlink_metadata(path) {
            Ok(_) => return self.os.read_link(path),
            Err(error) => self.missing(path, error)?,
        };
        match entry {
            Entry::Link(real) => Ok(real.to_path_buf()),
            Entry::Inside(real) => self.os.read_link(&real),
            Entry::Dir => Err(io::Error::from(io::ErrorKind::InvalidInput).into()),
        }
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.os.canonicalize(path).or_else(|error| match self.missing(path, error)? {
            Entry::Dir => Ok(path.to_path_buf()),
            Entry::Link(real) => self.os.canonicalize(real),
            Entry::Inside(real) => self.os.canonicalize(&real),
        })
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn file_system() -> LinkedFileSystem {
        let packages = [
            ("@acme/ui".to_string(), PathBuf::from("/repo/packages/ui")),
            ("tsconfig".to_string(), PathBuf::from("/repo/packages/tsconfig")),
        ];
        LinkedFileSystem::new(Path::new("/repo"), packages.into_iter().collect())
    }

    #[test]
    fn links_packages_into_the_root_node_modules() {
        let file_system = file_system();
        let entry = |path: &str| file_system.entry(Path::new(path));
        let ui = Path::new("/repo/packages/ui");

        assert_eq!(entry("/repo/node_modules"), Some(Entry::Dir));
        assert_eq!(entry("/repo/node_modules/@acme"), Some(Entry::Dir));
        assert_eq!(entry("/repo/node_modules/@acme/ui"), Some(Entry::Link(ui)));
        assert_eq!(
            entry("/repo/node_modules/@acme/ui/src/index.ts"),
            Some(Entry::Inside(ui.join("src/index.ts")))
        );
        assert_eq!(
            entry("/repo/node_modules/tsconfig/base.json"),
            Some(Entry::Inside(Path::new("/repo/packages/tsconfig").join("base.json")))
        );
    }

    #[test]
    fn links_nothing_else() {
        let file_system = file_system();
        for path in [
            "/repo/node_modules/@vendor",
            "/repo/node_modules/@acme/sdk",
            "/repo/node_modules/react",
            "/repo/packages/ui/node_modules/@acme/ui",
            "/node_modules/@acme/ui",
        ] {
            assert_eq!(file_system.entry(Path::new(path)), None, "{path}");
        }
    }

    #[test]
    fn only_missing_paths_are_linked() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("node_modules/@acme/ui")).unwrap();
        let packages = [
            ("@acme/ui".to_string(), root.join("packages/ui")),
            ("@acme/api".to_string(), root.join("packages/api")),
        ];
        let file_system = LinkedFileSystem::new(root, packages.into_iter().collect());
        // The error doesn't matter, only whether anything is there.
        let error = || io::Error::other("failed");

        let installed = root.join("node_modules/@acme/ui");
        assert_eq!(
            file_system.missing(&installed, error()).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        let linked = file_system.missing(&root.join("node_modules/@acme/api"), error()).unwrap();
        assert_eq!(linked, Entry::Link(&root.join("packages/api")));
    }
}
