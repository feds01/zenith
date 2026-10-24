//! A file system for other crates' tests, which reads a project from
//! somewhere other than the disk.

use std::{
    io,
    path::{Path, PathBuf},
};

use crate::fs::{DirEntry, FileKind, FileSystem, OsFileSystem};

/// The disk, with other contents for some files, and not [on the
/// disk](FileSystem::on_disk), as a commit of a repository isn't.
#[derive(Debug, Default)]
pub struct Overlay {
    /// The path and other contents of each file.
    files: Vec<(PathBuf, Vec<u8>)>,
}

impl Overlay {
    /// The disk, with `contents` for the file at `path`, as well as the files
    /// that it already has.
    pub fn with(mut self, path: PathBuf, contents: impl Into<Vec<u8>>) -> Self {
        self.files.push((path, contents.into()));
        self
    }

    /// The other contents of the file at `path`, if it has any.
    fn contents(&self, path: &Path) -> Option<&[u8]> {
        self.files.iter().find(|(file, _)| file == path).map(|(_, contents)| contents.as_slice())
    }
}

impl FileSystem for Overlay {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.contents(path) {
            Some(contents) => Ok(contents.to_vec()),
            None => OsFileSystem.read(path),
        }
    }

    fn kind(&self, path: &Path) -> io::Result<FileKind> {
        match self.contents(path) {
            Some(_) => Ok(FileKind::File),
            None => OsFileSystem.kind(path),
        }
    }

    fn link_kind(&self, path: &Path) -> io::Result<FileKind> {
        match self.contents(path) {
            Some(_) => Ok(FileKind::File),
            None => OsFileSystem.link_kind(path),
        }
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        OsFileSystem.read_dir(path)
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        OsFileSystem.read_link(path)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        OsFileSystem.canonicalize(path)
    }
}
