//! Reading files: the [`FileSystem`] that zenith reads a project through,
//! [`OsFileSystem`] for the disk, [`ListingCache`], which lists each
//! directory of another once, [`Recorder`], which records what is read
//! through another, and helpers that read the disk directly.

use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    ffi::OsString,
    fmt,
    fs::{self, File, FileType, Metadata},
    hash::{Hash, Hasher},
    io::{self, Read},
    mem,
    path::{Path, PathBuf, is_separator},
    sync::{
        Arc, LazyLock, Mutex, PoisonError, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use rustc_hash::FxHashSet;
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

    /// Whether the files are the disk's, so that another program, such as
    /// Cargo, sees them as zenith does.
    fn on_disk(&self) -> bool {
        false
    }

    /// Every file under the directory `dir`, if this file system holds a
    /// list of them, as a commit's tree does, so that they needn't be walked:
    /// each with what it is, without following links. `None` for one to
    /// walk, such as the disk.
    fn listed_files(&self, dir: &Path) -> Option<Vec<DirEntry>> {
        let _ = dir;
        None
    }

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

    /// Whether the directory of `path` lists an entry spelt exactly as its
    /// last component. Where names ignore case, as they do by default on
    /// macOS and Windows, `src/Config.rs` names the file `src/config.rs`,
    /// but isn't listed.
    fn is_listed(&self, path: &Path) -> bool {
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return false;
        };
        (self.read_dir(dir))
            .is_ok_and(|entries| entries.iter().any(|entry| entry.path.file_name() == Some(name)))
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// The disk, as a shared [`FileSystem`].
pub fn disk() -> &'static Arc<dyn FileSystem> {
    static DISK: LazyLock<Arc<dyn FileSystem>> = LazyLock::new(|| Arc::new(OsFileSystem));
    &DISK
}

/// Canonicalise a path: resolve symlinks and, on Windows, strip the `\\?\`
/// prefix where that is safe. A path that does not exist is normalised
/// lexically with [`normalize_path`] instead, so symlinks in its existing
/// ancestors stay unresolved.
pub fn adjust_canonicalisation<P: AsRef<Path>>(path: P) -> PathBuf {
    OsFileSystem.adjust_canonicalisation(path.as_ref())
}

/// Another [`FileSystem`], which lists each directory once to tell whether
/// it lists a path, for lookups that ask that of the same directories again
/// and again, as module lookups do. A directory's listing is kept for as long
/// as this lives, so it should live no longer than the lookups that share it.
/// Everything else is asked of the other file system.
#[derive(Debug)]
pub struct ListingCache<'a> {
    /// The other file system.
    file_system: &'a dyn FileSystem,

    /// The names of the entries of each directory listed so far, or `None`
    /// for one that couldn't be listed.
    listings: RwLock<HashMap<PathBuf, Option<Arc<HashSet<OsString>>>>>,
}

impl<'a> ListingCache<'a> {
    /// A cache of the listings of `file_system`'s directories.
    pub fn new(file_system: &'a dyn FileSystem) -> Self {
        Self { file_system, listings: RwLock::default() }
    }

    /// The names of the entries of the directory `dir`, listed the first time
    /// that they are asked for, or `None` if it can't be listed.
    fn names(&self, dir: &Path) -> Option<Arc<HashSet<OsString>>> {
        let listings = self.listings.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(names) = listings.get(dir) {
            return names.clone();
        }
        drop(listings);
        let names = self.file_system.read_dir(dir).ok().map(|entries| {
            let names =
                entries.into_iter().filter_map(|entry| entry.path.file_name().map(Into::into));
            Arc::new(names.collect())
        });
        let mut listings = self.listings.write().unwrap_or_else(PoisonError::into_inner);
        listings.entry(dir.to_path_buf()).or_insert(names).clone()
    }
}

impl FileSystem for ListingCache<'_> {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.file_system.read(path)
    }

    fn kind(&self, path: &Path) -> io::Result<FileKind> {
        self.file_system.kind(path)
    }

    fn link_kind(&self, path: &Path) -> io::Result<FileKind> {
        self.file_system.link_kind(path)
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        self.file_system.read_dir(path)
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        self.file_system.read_link(path)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.file_system.canonicalize(path)
    }

    fn on_disk(&self) -> bool {
        self.file_system.on_disk()
    }

    fn listed_files(&self, dir: &Path) -> Option<Vec<DirEntry>> {
        self.file_system.listed_files(dir)
    }

    fn read_text(&self, path: &Path) -> io::Result<String> {
        self.file_system.read_text(path)
    }

    fn read_json(&self, path: &Path) -> Result<Value, ReadJsonError> {
        self.file_system.read_json(path)
    }

    fn is_listed(&self, path: &Path) -> bool {
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return false;
        };
        self.names(dir).is_some_and(|names| names.contains(name))
    }
}

/// Another [`FileSystem`], which records what is asked of it while it
/// [records](Recorder::record), so that what is made from what was read can
/// be kept while that is unchanged. Everything is asked of the other file
/// system.
///
/// What is at a path under the project's root shows in the entries of the
/// directories above it, up to the root, since a link among them could lead
/// elsewhere, and what is at a path outside it only in what is there.
#[derive(Debug)]
pub struct Recorder {
    /// The other file system.
    file_system: Arc<dyn FileSystem>,

    /// The canonical root of the project.
    root: PathBuf,

    /// Whether it records.
    recording: AtomicBool,

    /// What was asked while it recorded, in parts that threads mostly don't
    /// share, so they seldom wait for one another.
    asked: Box<[Mutex<Asked>]>,
}

/// What a [`Recorder`] was asked about a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Question {
    /// What is there, if anything, without following a link at its end.
    Entry,

    /// Nothing: it was found not to be a directory, as a file isn't, so not
    /// one whose entries tell what is under it, since nothing is.
    NoDirectory,

    /// What is there, following links.
    Kind,

    /// The contents of the file there, or the entries of the directory.
    Contents,
}

/// What a [`Recorder`] was asked while it recorded.
#[derive(Debug, Default)]
struct Asked {
    /// The directories under the root, the root among them, that hold a path
    /// asked about.
    dirs: FxHashSet<Text>,

    /// The paths under the root that were found not to be directories.
    no_dirs: Vec<PathBuf>,

    /// The paths whose contents were read.
    contents: Vec<PathBuf>,

    /// The paths at which what is there was asked, following links, and
    /// those outside the root at which it was asked at all, each with what
    /// was there.
    kinds: Vec<(PathBuf, Seen)>,
}

/// What was at a path when it was asked about, following links, or `None`
/// for nothing.
type Seen = Option<FileKind>;

/// What was read through a [`Recorder`] while it recorded: what is at
/// these paths now tells whether it would read the same again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recorded {
    /// The paths whose contents were read, files or directories, and the
    /// directories under the root whose entries tell what is at a path
    /// asked about, in no particular order.
    pub contents: Vec<PathBuf>,

    /// The paths at which what is there was asked, following links, and
    /// those outside the root at which it was asked at all, each with what
    /// was there then, following links, or `None` for nothing, in no
    /// particular order. No directory that was recorded tells what is at
    /// them, so a value made from what was read depends on it being the
    /// same.
    pub kinds: Vec<(PathBuf, Option<FileKind>)>,
}

/// A path, told apart from others by its text, which is far quicker to hash
/// than its components, as a path is hashed.
#[derive(Debug)]
struct Text(PathBuf);

impl Text {
    /// The text of `path`.
    fn of(path: &Path) -> &[u8] {
        path.as_os_str().as_encoded_bytes()
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        Text::of(&self.0) == Text::of(&other.0)
    }
}

impl Eq for Text {}

impl Hash for Text {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Text::of(&self.0).hash(state);
    }
}

/// So that a path is looked up by its text, without a copy of it.
impl Borrow<[u8]> for Text {
    fn borrow(&self) -> &[u8] {
        Text::of(&self.0)
    }
}

impl Recorded {
    /// Add what `other` recorded, which may name the same paths.
    pub fn merge(&mut self, other: &Self) {
        self.contents.extend(other.contents.iter().cloned());
        self.kinds.extend(other.kinds.iter().cloned());
    }
}

impl Recorder {
    /// The parts of what was asked, which threads share when there are more.
    const PARTS: usize = 64;

    /// A recorder of what is read from `file_system` of the project at the
    /// canonical `root`.
    pub fn new(file_system: Arc<dyn FileSystem>, root: &Path) -> Self {
        Self {
            file_system,
            root: root.to_path_buf(),
            recording: AtomicBool::new(false),
            asked: (0..Self::PARTS).map(|_| Mutex::default()).collect(),
        }
    }

    /// Run `work`, recording what is read through this file system
    /// meanwhile, by any thread. Nothing else should read through it then.
    pub fn record<R>(&self, work: impl FnOnce() -> R) -> (R, Recorded) {
        self.recording.store(true, Ordering::Relaxed);
        let result = work();
        self.recording.store(false, Ordering::Relaxed);
        (result, self.take())
    }

    /// Run `work` without recording what it reads, for what a value made
    /// from what is recorded doesn't depend on.
    pub fn unrecorded<R>(&self, work: impl FnOnce() -> R) -> R {
        let recording = self.recording.swap(false, Ordering::Relaxed);
        let result = work();
        self.recording.store(recording, Ordering::Relaxed);
        result
    }

    /// Whether it records.
    fn records(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Note that `question` was asked about `path`, if it records, with what
    /// was `seen` there if that matters.
    fn ask(&self, question: Question, path: &Path, seen: impl FnOnce() -> Seen) {
        if !self.records() {
            return;
        }
        let parent = path.parent().filter(|parent| self.is_under_root(parent));
        let seen = match (question, parent) {
            (Question::Kind, _) | (Question::Entry | Question::NoDirectory, None) => Some(seen()),
            _ => None,
        };
        let part = rayon::current_thread_index().map_or(0, |index| index + 1) % Self::PARTS;
        let mut asked = self.asked[part].lock().unwrap_or_else(PoisonError::into_inner);
        // Most paths asked about share their directory with others.
        if let Some(parent) = parent
            && !asked.dirs.contains(Text::of(parent))
        {
            asked.dirs.insert(Text(parent.to_path_buf()));
        }
        match (question, seen) {
            (Question::Contents, _) => asked.contents.push(path.to_path_buf()),
            (_, Some(seen)) => asked.kinds.push((path.to_path_buf(), seen)),
            (Question::NoDirectory, None) => asked.no_dirs.push(path.to_path_buf()),
            (Question::Entry | Question::Kind, None) => {}
        }
    }

    /// Note what was `found` at `path`: a file, or something under what
    /// isn't a directory.
    fn found(&self, path: &Path, found: &io::Result<FileKind>) {
        match found {
            Ok(FileKind::File | FileKind::Other) => {
                self.ask(Question::NoDirectory, path, || Some(FileKind::File));
            }
            Err(error) if is_under_no_directory(error) => {
                if let Some(parent) = path.parent() {
                    self.ask(Question::NoDirectory, parent, || self.seen(parent));
                }
            }
            _ => {}
        }
    }

    /// What is at `path`, following links, or `None` for nothing.
    fn seen(&self, path: &Path) -> Seen {
        self.file_system.kind(path).ok()
    }

    /// What was recorded, which is then forgotten.
    fn take(&self) -> Recorded {
        // 1. Gather what each thread was asked.
        let mut all = Asked::default();
        for part in &self.asked {
            let asked = mem::take(&mut *part.lock().unwrap_or_else(PoisonError::into_inner));
            all.dirs.extend(asked.dirs);
            all.no_dirs.extend(asked.no_dirs);
            all.contents.extend(asked.contents);
            all.kinds.extend(asked.kinds);
        }

        // 2. Add the directories above those, up to the root. Each one's are added with
        //    it, so the climb stops at one that is already there.
        let mut above = FxHashSet::default();
        for Text(dir) in &all.dirs {
            for dir in dir.ancestors().skip(1).take_while(|dir| self.is_under_root(dir)) {
                if all.dirs.contains(Text::of(dir)) || above.contains(Text::of(dir)) {
                    break;
                }
                above.insert(Text(dir.to_path_buf()));
            }
        }

        // 3. A path found not to be a directory holds nothing, so isn't one to record.
        let unique =
            |paths: Vec<PathBuf>| -> FxHashSet<Text> { paths.into_iter().map(Text).collect() };
        let no_dirs = unique(all.no_dirs);
        let dirs = (all.dirs.into_iter().chain(above)).filter(|dir| !no_dirs.contains(dir));
        let mut contents = unique(all.contents);
        contents.extend(dirs);
        let kinds: FxHashSet<(Text, Seen)> =
            all.kinds.into_iter().map(|(path, seen)| (Text(path), seen)).collect();
        Recorded {
            contents: contents.into_iter().map(|Text(path)| path).collect(),
            kinds: kinds.into_iter().map(|(Text(path), seen)| (path, seen)).collect(),
        }
    }

    /// Whether `dir` is the root or under it, told by their text, as quickly
    /// as the many paths that a resolver asks about need.
    fn is_under_root(&self, dir: &Path) -> bool {
        let root = self.root.as_os_str().as_encoded_bytes();
        let rest = dir.as_os_str().as_encoded_bytes().strip_prefix(root);
        rest.is_some_and(|rest| match rest.first() {
            None => true,
            Some(next) => {
                root.last().is_some_and(|last| is_separator(char::from(*last)))
                    || is_separator(char::from(*next))
            }
        })
    }
}

impl FileSystem for Recorder {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.ask(Question::Contents, path, || None);
        self.file_system.read(path)
    }

    fn kind(&self, path: &Path) -> io::Result<FileKind> {
        let kind = self.file_system.kind(path);
        self.ask(Question::Kind, path, || kind.as_ref().ok().copied());
        self.found(path, &kind);
        kind
    }

    fn link_kind(&self, path: &Path) -> io::Result<FileKind> {
        let kind = self.file_system.link_kind(path);
        self.ask(Question::Entry, path, || match &kind {
            Ok(FileKind::Symlink) => self.seen(path),
            kind => kind.as_ref().ok().copied(),
        });
        self.found(path, &kind);
        kind
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        self.ask(Question::Contents, path, || None);
        self.file_system.read_dir(path)
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        self.ask(Question::Entry, path, || self.seen(path));
        self.file_system.read_link(path)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let canonical = self.file_system.canonicalize(path);
        self.ask(Question::Kind, path, || canonical.as_ref().ok().and_then(|_| self.seen(path)));
        canonical
    }

    fn on_disk(&self) -> bool {
        self.file_system.on_disk()
    }

    fn listed_files(&self, dir: &Path) -> Option<Vec<DirEntry>> {
        self.file_system.listed_files(dir)
    }

    fn read_text(&self, path: &Path) -> io::Result<String> {
        self.ask(Question::Contents, path, || None);
        self.file_system.read_text(path)
    }
}

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
        // Without the `\\?\` prefix on Windows, where that is safe.
        Ok(dunce::simplified(&fs::read_link(path)?).to_path_buf())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        // Without the `\\?\` prefix on Windows, where that is safe.
        dunce::canonicalize(path)
    }

    fn read_text(&self, path: &Path) -> io::Result<String> {
        read_text(path)
    }

    fn on_disk(&self) -> bool {
        true
    }
}

/// Whether `error`, from asking about a path, says that what would hold it
/// isn't a directory. Windows says so for a missing directory too, with an
/// error that only tells that the path's directory wasn't found.
fn is_under_no_directory(error: &io::Error) -> bool {
    /// The error of Windows for a path whose directory wasn't found.
    const ERROR_PATH_NOT_FOUND: i32 = 3;
    match error.kind() {
        io::ErrorKind::NotADirectory => true,
        io::ErrorKind::NotFound => {
            cfg!(windows) && error.raw_os_error() == Some(ERROR_PATH_NOT_FOUND)
        }
        _ => false,
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

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_recorder_records_what_is_read_while_it_records() {
        let dir = TempDir::new().unwrap();
        let outside = adjust_canonicalisation(dir.path());
        let root = outside.join("project");
        fs::create_dir_all(root.join("src/lib")).unwrap();
        fs::write(root.join("src/a.ts"), "").unwrap();
        fs::write(root.join("package.json"), "{}").unwrap();
        let recorder = Recorder::new(disk().clone(), &root);

        // Nothing is recorded but what is read while it records.
        recorder.link_kind(&root.join("b.ts")).unwrap_err();
        let ((), recorded) = recorder.record(|| {
            // What is at a path under the root is in the directories above it,
            // and nothing is under a file.
            recorder.link_kind(&root.join("src/lib/missing.ts")).unwrap_err();
            recorder.link_kind(&root.join("src/a.ts/tsconfig.json")).unwrap_err();
            // Contents are recorded themselves, as are what is at a path,
            // following links, and what is at one outside the root.
            recorder.read(&root.join("package.json")).unwrap();
            recorder.kind(&root.join("src")).unwrap();
            recorder.link_kind(&outside.join("node_modules")).unwrap_err();
        });
        let sorted = |mut paths: Vec<PathBuf>| {
            paths.sort();
            paths
        };
        let contents = ["", "package.json", "src", "src/lib"].map(|path| root.join(path));
        assert_eq!(sorted(recorded.contents), contents);
        let kinds =
            [(outside.join("node_modules"), None), (root.join("src"), Some(FileKind::Directory))];
        let mut recorded_kinds = recorded.kinds;
        recorded_kinds.sort_by(|(a, _), (b, _)| a.cmp(b));
        assert_eq!(recorded_kinds, kinds);

        // What it recorded is forgotten, and what is read unrecorded never noted.
        let ((), recorded) = recorder.record(|| {
            recorder.unrecorded(|| recorder.read(&root.join("package.json")).unwrap());
        });
        assert_eq!(recorded, Recorded::default());
    }

    #[test]
    fn a_listing_cache_keeps_each_listing() {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        fs::write(root.join("config.rs"), "").unwrap();
        let cache = ListingCache::new(&OsFileSystem);

        // Spelt as it is listed, and only so, as on the disk.
        assert!(cache.is_listed(&root.join("config.rs")));
        assert!(!cache.is_listed(&root.join("Config.rs")));
        assert!(!cache.is_listed(&root.join("missing/config.rs")));
        assert_eq!(
            cache.is_listed(&root.join("Config.rs")),
            OsFileSystem.is_listed(&root.join("Config.rs"))
        );

        // A file added later isn't listed, as the directory was listed before.
        fs::write(root.join("later.rs"), "").unwrap();
        assert!(!cache.is_listed(&root.join("later.rs")));
        assert!(ListingCache::new(&OsFileSystem).is_listed(&root.join("later.rs")));
        assert!(cache.is_file(&root.join("later.rs")));
    }

    #[test]
    fn existing_paths_are_canonicalised() {
        assert_eq!(adjust_canonicalisation("."), env::current_dir().unwrap());
    }

    #[test]
    fn missing_paths_are_normalised() {
        let cwd = env::current_dir().unwrap();
        assert_eq!(adjust_canonicalisation("no-such-dir/x/../y.ts"), cwd.join("no-such-dir/y.ts"));
    }

    #[test]
    fn strips_a_byte_order_mark() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.json");
        fs::write(&path, "\u{feff}{ \"name\": \"web\" }").unwrap();

        assert_eq!(read_text(&path).unwrap(), "{ \"name\": \"web\" }");
        assert_eq!(OsFileSystem.read_json(&path).unwrap(), json!({ "name": "web" }));
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
            OsFileSystem.read_json(&path),
            Err(ReadJsonError::Read(error)) if error.kind() == io::ErrorKind::NotFound
        ));

        fs::write(&path, "{ not json").unwrap();
        let error = OsFileSystem.read_json(&path).unwrap_err();
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
