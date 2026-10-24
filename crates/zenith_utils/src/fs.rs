//! Reading files: the [`FileSystem`] that zenith reads a project through,
//! [`OsFileSystem`] for the disk, and helpers that read the disk directly.

use std::{
    fmt,
    fs::{self, File, FileType, Metadata},
    io::{self, Read},
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::path::normalize_path;

/// The files of a project, as zenith reads them: the disk, through
/// [`OsFileSystem`], or another source, such as a commit of a git repository.
/// Paths are absolute.
pub trait FileSystem: fmt::Debug + Send + Sync {
    /// The contents of the file at `path`.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// What `path` names, following links.
    fn kind(&self, path: &Path) -> io::Result<FileKind>;

    /// What `path` names, without following a link at its end.
    fn link_kind(&self, path: &Path) -> io::Result<FileKind>;

    /// The entries of the directory at `path`, in no particular order.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>>;

    /// What the link at `path` points to.
    fn read_link(&self, path: &Path) -> io::Result<PathBuf>;

    /// The canonical form of `path`, which must name something: absolute,
    /// with every link resolved.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;

    /// The file at `path` as text, without a leading byte order mark, as
    /// [`read_text`] reads it.
    fn read_text(&self, path: &Path) -> io::Result<String> {
        let contents = String::from_utf8(self.read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(without_byte_order_mark(contents))
    }

    /// The JSON file at `path`, read with [`Self::read_text`], and parsed.
    fn read_json(&self, path: &Path) -> Result<Value, ReadJsonError> {
        let contents = self.read_text(path).map_err(ReadJsonError::Read)?;
        serde_json::from_str(&contents).map_err(ReadJsonError::Parse)
    }

    /// Whether `path` names a file, following links.
    fn is_file(&self, path: &Path) -> bool {
        matches!(self.kind(path), Ok(FileKind::File))
    }

    /// Whether `path` names a directory, following links.
    fn is_dir(&self, path: &Path) -> bool {
        matches!(self.kind(path), Ok(FileKind::Directory))
    }

    /// The nearest file named `name` in `start` or one of its ancestors, such
    /// as the configuration file that applies to a directory. Directories
    /// with that name are skipped. A relative `start` is taken from the
    /// current directory.
    fn find_in_ancestors(&self, start: &Path, name: &str) -> Option<PathBuf> {
        normalize_path(start).ancestors().map(|dir| dir.join(name)).find(|path| self.is_file(path))
    }

    /// The canonical form of `path`, as [`Self::canonicalize`] gives it, or,
    /// if it names nothing, `path` normalised lexically with
    /// [`normalize_path`], so that links in its ancestors stay unresolved.
    fn adjust_canonicalisation(&self, path: &Path) -> PathBuf {
        self.canonicalize(path).unwrap_or_else(|_| normalize_path(path))
    }
}

/// What a path names, as a [`FileSystem`] tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A file.
    File,

    /// A directory.
    Directory,

    /// A symbolic link, which is only seen where links aren't followed.
    Symlink,

    /// Anything else, such as a socket.
    Other,
}

impl From<FileType> for FileKind {
    fn from(file_type: FileType) -> Self {
        if file_type.is_symlink() {
            Self::Symlink
        } else if file_type.is_dir() {
            Self::Directory
        } else if file_type.is_file() {
            Self::File
        } else {
            Self::Other
        }
    }
}

/// An entry of a directory, as [`FileSystem::read_dir`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// The path of the entry: the directory's, joined with its name.
    pub path: PathBuf,

    /// What the entry is, without following a link.
    pub kind: FileKind,
}

/// The disk, as the operating system shows it.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsFileSystem;

impl FileSystem for OsFileSystem {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn kind(&self, path: &Path) -> io::Result<FileKind> {
        Ok(fs::metadata(path)?.file_type().into())
    }

    fn link_kind(&self, path: &Path) -> io::Result<FileKind> {
        Ok(fs::symlink_metadata(path)?.file_type().into())
    }

    /// The entries that can be read; one that can't be is left out.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let entries = fs::read_dir(path)?.flatten().filter_map(|entry| {
            Some(DirEntry { path: entry.path(), kind: entry.file_type().ok()?.into() })
        });
        Ok(entries.collect())
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        fs::read_link(path)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        // Without the `\\?\` prefix on Windows where that is safe.
        dunce::canonicalize(path)
    }

    fn read_text(&self, path: &Path) -> io::Result<String> {
        read_text(path)
    }
}

/// Read a text file, without a leading byte order mark, which editors on
/// Windows often write and which npm, pnpm and yarn all accept.
pub fn read_text(path: &Path) -> io::Result<String> {
    read_text_from(&mut File::open(path)?)
}

/// Read what is left of `reader` as text, without a leading byte order
/// mark, as [`read_text`] does, e.g. a file whose metadata was read first.
pub fn read_text_from(reader: &mut impl Read) -> io::Result<String> {
    Ok(without_byte_order_mark(io::read_to_string(reader)?))
}

/// `contents` without a leading byte order mark.
fn without_byte_order_mark(contents: String) -> String {
    match contents.strip_prefix('\u{feff}') {
        Some(contents) => contents.to_string(),
        None => contents,
    }
}

/// Read a text file, as [`read_text`] does, with its metadata, taken before
/// it is read, or `None` if that couldn't be taken. Opening the file once for
/// both is faster than looking it up twice.
pub fn read_text_with_metadata(path: &Path) -> io::Result<(String, Option<Metadata>)> {
    let mut file = File::open(path)?;
    let metadata = file.metadata().ok();
    Ok((read_text_from(&mut file)?, metadata))
}

/// Why a JSON file couldn't be read.
#[derive(Debug)]
pub enum ReadJsonError {
    /// The file couldn't be read.
    Read(io::Error),

    /// The file isn't valid JSON.
    Parse(serde_json::Error),
}

impl fmt::Display for ReadJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "{error}"),
            Self::Parse(error) => write!(f, "invalid JSON: {error}"),
        }
    }
}

impl std::error::Error for ReadJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Parse(error) => Some(error),
        }
    }
}

/// Read a JSON file from the disk, as [`FileSystem::read_json`] does.
pub fn read_json(path: &Path) -> Result<Value, ReadJsonError> {
    OsFileSystem.read_json(path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn strips_a_byte_order_mark() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.json");
        fs::write(&path, "\u{feff}{ \"name\": \"web\" }").unwrap();

        assert_eq!(read_text(&path).unwrap(), "{ \"name\": \"web\" }");
        assert_eq!(read_json(&path).unwrap(), json!({ "name": "web" }));
    }

    #[test]
    fn reads_text_from_an_open_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.ts");
        fs::write(&path, "\u{feff}import a from './a';\n").unwrap();
        let mut file = File::open(&path).unwrap();
        assert_eq!(read_text_from(&mut file).unwrap(), "import a from './a';\n");

        fs::write(&path, b"\xff").unwrap();
        let error = read_text_from(&mut File::open(&path).unwrap()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), read_text(&path).unwrap_err().to_string());
    }

    #[test]
    fn tells_missing_files_from_invalid_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.json");
        assert!(matches!(
            read_json(&path),
            Err(ReadJsonError::Read(error)) if error.kind() == io::ErrorKind::NotFound
        ));

        fs::write(&path, "{ not json").unwrap();
        let error = read_json(&path).unwrap_err();
        assert!(matches!(error, ReadJsonError::Parse(_)));
        assert!(error.to_string().starts_with("invalid JSON: "), "{error}");
    }

    #[test]
    fn finds_the_nearest_file_in_or_above_a_directory() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let nested = root.join("web/src/app");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("a.toml"), "").unwrap();

        assert_eq!(OsFileSystem.find_in_ancestors(root, "a.toml"), Some(root.join("a.toml")));
        assert_eq!(OsFileSystem.find_in_ancestors(&nested, "a.toml"), Some(root.join("a.toml")));
        assert_eq!(OsFileSystem.find_in_ancestors(&nested, "b.toml"), None);

        fs::write(root.join("web/a.toml"), "").unwrap();
        assert_eq!(
            OsFileSystem.find_in_ancestors(&nested, "a.toml"),
            Some(root.join("web/a.toml"))
        );
        assert_eq!(OsFileSystem.find_in_ancestors(root, "a.toml"), Some(root.join("a.toml")));
    }

    #[test]
    fn skips_directories_with_the_name() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("web/a.toml")).unwrap();
        fs::write(root.join("a.toml"), "").unwrap();
        assert_eq!(
            OsFileSystem.find_in_ancestors(&root.join("web"), "a.toml"),
            Some(root.join("a.toml"))
        );
    }

    /// A file system of one file, `a.json` in the root, for the methods that
    /// the trait provides. The root is any path without a parent, such as
    /// `C:\` on Windows.
    #[derive(Debug)]
    struct OneFile(&'static [u8]);

    impl OneFile {
        /// Whether `path` names the file.
        fn is_the_file(path: &Path) -> bool {
            path.file_name().is_some_and(|name| name == "a.json")
                && path.parent().is_some_and(|parent| parent.parent().is_none())
        }
    }

    impl FileSystem for OneFile {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            match Self::is_the_file(path) {
                true => Ok(self.0.to_vec()),
                false => Err(io::ErrorKind::NotFound.into()),
            }
        }

        fn kind(&self, path: &Path) -> io::Result<FileKind> {
            self.link_kind(path)
        }

        fn link_kind(&self, path: &Path) -> io::Result<FileKind> {
            match path.parent() {
                None => Ok(FileKind::Directory),
                Some(_) if Self::is_the_file(path) => Ok(FileKind::File),
                Some(_) => Err(io::ErrorKind::NotFound.into()),
            }
        }

        fn read_dir(&self, _: &Path) -> io::Result<Vec<DirEntry>> {
            Ok(vec![DirEntry { path: PathBuf::from("/a.json"), kind: FileKind::File }])
        }

        fn read_link(&self, _: &Path) -> io::Result<PathBuf> {
            Err(io::ErrorKind::InvalidInput.into())
        }

        fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
            self.kind(path).map(|_| path.to_owned())
        }
    }

    #[test]
    fn a_file_system_reads_text_and_json_from_its_bytes() {
        let file_system = OneFile("\u{feff}{ \"name\": \"web\" }".as_bytes());
        let path = Path::new("/a.json");
        assert_eq!(file_system.read_text(path).unwrap(), "{ \"name\": \"web\" }");
        assert_eq!(file_system.read_json(path).unwrap(), json!({ "name": "web" }));
        assert!(file_system.is_file(path));
        assert!(file_system.is_dir(Path::new("/")));
        assert!(!file_system.is_file(Path::new("/b.json")));

        let error = OneFile(b"\xff").read_text(path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_file_system_finds_files_in_ancestors_and_canonicalises_what_exists() {
        let file_system = OneFile(b"{}");
        let found = file_system.find_in_ancestors(Path::new("/web/src"), "a.json");
        assert_eq!(found, Some(normalize_path("/a.json")));
        assert_eq!(file_system.find_in_ancestors(Path::new("/web"), "b.json"), None);
        let missing = Path::new("/web/../b.json");
        assert_eq!(file_system.adjust_canonicalisation(missing), normalize_path(missing));
    }

    #[cfg(unix)]
    #[test]
    fn the_disk_tells_files_directories_and_links_apart() {
        let dir = TempDir::new().unwrap();
        let root = OsFileSystem.canonicalize(dir.path()).unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "export {};\n").unwrap();
        std::os::unix::fs::symlink("src", root.join("link")).unwrap();

        let os = OsFileSystem;
        assert_eq!(os.read(&root.join("src/a.ts")).unwrap(), b"export {};\n");
        assert_eq!(os.kind(&root.join("src")).unwrap(), FileKind::Directory);
        assert_eq!(os.kind(&root.join("link")).unwrap(), FileKind::Directory);
        assert_eq!(os.link_kind(&root.join("link")).unwrap(), FileKind::Symlink);
        assert_eq!(os.read_link(&root.join("link")).unwrap(), Path::new("src"));
        assert_eq!(os.canonicalize(&root.join("link/a.ts")).unwrap(), root.join("src/a.ts"));
        assert!(os.kind(&root.join("b.ts")).is_err());

        let mut entries = os.read_dir(&root).unwrap();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            entries,
            [
                DirEntry { path: root.join("link"), kind: FileKind::Symlink },
                DirEntry { path: root.join("src"), kind: FileKind::Directory },
            ]
        );
    }
}
