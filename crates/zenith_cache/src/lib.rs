//! The cache of what zenith found in each file on an earlier run, so that
//! files that haven't changed since are neither read nor processed again.
//!
//! A [`Cache`] keeps one [`Cached`] value for each file, such as what the
//! parser found in it, in one file in the cache directory, which a stage
//! reads at its start and rewrites at its end if anything changed. Each entry
//! is keyed by the file's name, and is stamped with the file's
//! [`Fingerprint`], its size, times and inode, and the hash of its contents.
//! A file's value is reused:
//!
//! - **unread**, if its fingerprint is unchanged, and old enough to trust: its
//!   modification time is more than [`RACY_WINDOW`] before the run that
//!   recorded it started. A file changed within that window of being read, in
//!   the same tick of a coarse clock, could otherwise keep its fingerprint, as
//!   git's racy-clean entries can.
//! - **read but not processed**, if its fingerprint changed or is too recent,
//!   but its contents hash the same, e.g. after a checkout or a `touch`.
//!
//! Anything else is computed again. The cache never changes what a run
//! finds: a file whose stamp doesn't match, or whose value doesn't decode, is
//! processed as if there were no cache, and a cache file that is missing,
//! corrupt, or written by another version of zenith or for other inputs is
//! ignored, and replaced at the end of the run.
//!
//! Concurrent runs are safe: a run reads the whole file at once, and
//! replaces it by renaming a complete temporary file over it, so the file is
//! always one run's cache, whichever wrote last.

mod encoding;

use std::{
    fs::{self, Metadata},
    hash::{Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use encoding::Encoded;
pub use encoding::{Cached, Decoder, Encoder};
use log::debug;
use rustc_hash::FxHashMap;
use tempfile::NamedTempFile;
use xxhash_rust::xxh3::{Xxh3, xxh3_128};
use zenith_utils::fs::{FileSystem, read_text_with_metadata};

use crate::encoding::Rejected;

/// How long before a run starts a file must have last changed for its
/// fingerprint to be trusted on the next run. This covers the coarsest
/// modification times in common use, FAT's two seconds.
const RACY_WINDOW: Duration = Duration::from_secs(2);

/// Where to keep the cache, and the version of zenith that it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSettings {
    /// The directory of the cache. It is created when the cache is first
    /// written, with a `.gitignore` that ignores everything in it.
    pub dir: PathBuf,

    /// The version of zenith, which a cache file must have been written by
    /// to be used. What zenith finds may change between versions, so a new
    /// version starts from an empty cache.
    pub version: String,
}

impl CacheSettings {
    /// The path of the cache file of `T`s.
    pub fn path<T: Cached>(&self) -> PathBuf {
        self.dir.join(T::FILE)
    }
}

/// What a file's metadata says about its contents. If it is unchanged, the
/// contents almost certainly are too.
///
/// The change time and inode are only known on Unix. On Windows, the
/// creation time stands in for the change time, and the inode is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    /// The size of the file, in bytes.
    len: u64,

    /// When the file was last modified, in nanoseconds since the Unix epoch.
    modified: u64,

    /// When the file's metadata last changed (Unix), or when it was created
    /// (Windows), in a platform-specific unit. Unlike the modification time,
    /// the change time can't be set back.
    changed: u64,

    /// The inode of the file, which changes when it is replaced.
    inode: u64,
}

impl Fingerprint {
    /// The fingerprint of a file with `metadata`, or `None` if its
    /// modification time is unknown, or before the Unix epoch.
    fn new(metadata: &Metadata) -> Option<Self> {
        let modified = nanos_since_epoch(metadata.modified().ok()?)?;
        #[cfg(unix)]
        let (changed, inode) = {
            use std::os::unix::fs::MetadataExt;
            let changed = (metadata.ctime() as u64)
                .wrapping_mul(1_000_000_000)
                .wrapping_add(metadata.ctime_nsec() as u64);
            (changed, metadata.ino())
        };
        #[cfg(windows)]
        let (changed, inode) = {
            use std::os::windows::fs::MetadataExt;
            (metadata.creation_time(), 0)
        };
        #[cfg(not(any(unix, windows)))]
        let (changed, inode) = (0, 0);
        Some(Self { len: metadata.len(), modified, changed, inode })
    }
}

/// When a file was last seen, as its cache entry records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// The fingerprint of the file's metadata, taken before it was read, or
    /// `None` if it couldn't be taken.
    fingerprint: Option<Fingerprint>,

    /// The XXH3 hash of the file's contents.
    hash: u128,
}

/// A file's entry in the cache.
#[derive(Debug)]
pub struct Entry<T> {
    /// When the file was last seen.
    stamp: Stamp,

    /// What was found in it, decoded when it is used.
    value: Encoded<T>,
}

/// Where a file's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The cache, without reading the file, since its fingerprint was
    /// unchanged.
    Unchanged,

    /// The cache, after reading the file and finding that its contents were
    /// unchanged.
    Verified,

    /// The computation, since the file had no entry, or had changed.
    Computed,
}

/// A file's value, from the cache or computed.
#[derive(Debug)]
pub struct Lookup<T> {
    /// The value, or why the file couldn't be read.
    pub value: io::Result<T>,

    /// The file's stamp, for its entry in the next cache, or `None` if it
    /// couldn't be read, or there is no cache.
    pub stamp: Option<Stamp>,

    /// Where `value` came from.
    pub origin: Origin,
}

impl<T> Lookup<T> {
    /// A value computed without a cache.
    pub fn uncached(value: io::Result<T>) -> Self {
        Self { value, stamp: None, origin: Origin::Computed }
    }
}

/// The cache of `T`s that an earlier run wrote, which a stage takes apart
/// file by file, and then the cache it writes for the next run.
#[derive(Debug)]
pub struct Cache<T> {
    /// Where the cache is kept.
    settings: CacheSettings,

    /// The hash of the version and the inputs of the values, which the cache
    /// file must have been written with.
    key: u64,

    /// When the run that wrote the cache started, in nanoseconds since the
    /// Unix epoch.
    written_at: u64,

    /// The entries that haven't been taken yet, by the name of their file.
    entries: FxHashMap<String, Entry<T>>,
}

impl<T: Cached> Cache<T> {
    /// Read the cache of `settings`, of values computed with `inputs`, such
    /// as the options of the parser. Every input is hashed into the key,
    /// rather than named, so a new one can't be left out. A cache file that
    /// is missing, corrupt, or written by another version of zenith or with
    /// other inputs is an empty cache.
    pub fn load(settings: CacheSettings, inputs: impl Hash) -> Self {
        let mut hasher = Xxh3::new();
        (settings.version.as_str(), inputs).hash(&mut hasher);
        let mut cache =
            Self { key: hasher.finish(), settings, written_at: 0, entries: FxHashMap::default() };

        let path = cache.settings.path::<T>();
        let decoded = fs::read(&path)
            .map_err(|error| debug!("no cache at `{}`: {error}", path.display()))
            .and_then(|bytes| {
                encoding::decode(bytes, cache.key).map_err(|rejected| match rejected {
                    Rejected::OtherKey => debug!(
                        "ignoring the cache at `{}`, of another version of zenith or other inputs",
                        path.display()
                    ),
                    Rejected::Corrupt => {
                        debug!("ignoring the cache at `{}`, which is corrupt", path.display())
                    }
                })
            });
        if let Ok(decoded) = decoded {
            debug!("read the cache of {} files from `{}`", decoded.entries.len(), path.display());
            (cache.written_at, cache.entries) = (decoded.written_at, decoded.entries);
        }
        cache
    }

    /// Where the cache is kept.
    pub fn settings(&self) -> &CacheSettings {
        &self.settings
    }

    /// The number of entries that haven't been taken.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether every entry has been taken.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Take the entry of the file `name` out of the cache.
    pub fn take(&mut self, name: &str) -> Option<Entry<T>> {
        self.entries.remove(name)
    }

    /// The value of the file at `path` in `file_system`: from `entry`, its
    /// entry in this cache, if that shows the file is unchanged, or else from
    /// `compute`, given its contents. Off the disk, a file has no metadata to
    /// trust, so it is always read, and its entry used if its contents hash
    /// the same. An entry whose value doesn't decode is not used.
    pub fn lookup(
        &self,
        file_system: &dyn FileSystem,
        path: &Path,
        entry: Option<Entry<T>>,
        compute: impl FnOnce(&str) -> T,
    ) -> Lookup<T> {
        // 1. Reuse the entry without reading the file if its fingerprint is unchanged
        //    and old enough to trust.
        let on_disk = file_system.on_disk();
        let entry = match entry {
            Some(entry) if on_disk && self.is_unchanged(path, entry.stamp) => {
                match entry.value.decode() {
                    Some(value) => {
                        let (stamp, origin) = (Some(entry.stamp), Origin::Unchanged);
                        return Lookup { value: Ok(value), stamp, origin };
                    }
                    None => None,
                }
            }
            entry => entry,
        };

        // 2. Otherwise read the file, taking its fingerprint first, so that a change
        //    made while it is read makes it differ next time.
        let read = match on_disk {
            true => read_text_with_metadata(path),
            false => file_system.read_text(path).map(|source| (source, None)),
        };
        let (source, metadata) = match read {
            Ok(read) => read,
            Err(error) => return Lookup::uncached(Err(error)),
        };
        let fingerprint = metadata.as_ref().and_then(Fingerprint::new);
        let stamp = Stamp { fingerprint, hash: xxh3_128(source.as_bytes()) };

        // 3. Reuse the entry if the contents hash the same, and compute the value
        //    otherwise.
        let cached = entry.filter(|entry| entry.stamp.hash == stamp.hash);
        let (value, origin) = match cached.and_then(|entry| entry.value.decode()) {
            Some(value) => (value, Origin::Verified),
            None => (compute(&source), Origin::Computed),
        };
        Lookup { value: Ok(value), stamp: Some(stamp), origin }
    }

    /// Whether the file at `path`, last seen with `stamp`, is unchanged by
    /// its metadata alone: its fingerprint is the same, and was last modified
    /// well before the run that recorded it started.
    fn is_unchanged(&self, path: &Path, stamp: Stamp) -> bool {
        let Some(recorded) = stamp.fingerprint else {
            return false;
        };
        let trusted = recorded.modified.saturating_add(RACY_WINDOW.as_nanos() as u64);
        trusted < self.written_at
            && fs::metadata(path).ok().as_ref().and_then(Fingerprint::new) == Some(recorded)
    }

    /// Write the cache of the run that started at `started`, with an entry
    /// for each of `entries`, replacing the cache file.
    pub fn save<'a>(
        &self,
        started: SystemTime,
        entries: impl ExactSizeIterator<Item = (&'a str, &'a Stamp, &'a T)>,
    ) -> io::Result<()>
    where
        T: 'a,
    {
        // 1. Create the directory, which git should ignore wherever it is.
        let dir = &self.settings.dir;
        fs::create_dir_all(dir)?;
        let gitignore = dir.join(".gitignore");
        if !gitignore.exists() {
            fs::write(gitignore, "# Created by zenith.\n*\n")?;
        }

        // 2. Write the whole file next to the cache file, then rename it over it, so
        //    that other runs read either the old cache or the new one.
        let written_at = nanos_since_epoch(started).unwrap_or(0);
        let bytes = encoding::encode(self.key, written_at, entries);
        let mut file = NamedTempFile::new_in(dir)?;
        file.write_all(&bytes)?;
        file.persist(self.settings.path::<T>()).map_err(|error| error.error)?;
        Ok(())
    }
}

/// The nanoseconds from the Unix epoch to `time`, or `None` if it is before
/// it.
fn nanos_since_epoch(time: SystemTime) -> Option<u64> {
    let since = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    u64::try_from(since.as_nanos()).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;
    use zenith_utils::{fs::OsFileSystem, testing::Overlay};

    use super::*;
    use crate::encoding::tests::Value;

    /// A file with `source` in a new temporary directory, and its
    /// fingerprint.
    fn file(source: &str) -> (TempDir, PathBuf, Fingerprint) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.ts");
        fs::write(&path, source).unwrap();
        let fingerprint = Fingerprint::new(&fs::metadata(&path).unwrap()).unwrap();
        (dir, path, fingerprint)
    }

    /// The settings of a cache in `dir`, for version `1.0.0`.
    fn settings(dir: &Path) -> CacheSettings {
        CacheSettings { dir: dir.join("cache"), version: "1.0.0".into() }
    }

    /// An empty cache, written `after` the file with `fingerprint` was last
    /// modified.
    fn cache_written(fingerprint: Fingerprint, after: Duration) -> Cache<Value> {
        let written_at = fingerprint.modified + after.as_nanos() as u64;
        let settings = settings(Path::new("unused"));
        Cache { settings, key: 0, written_at, entries: FxHashMap::default() }
    }

    /// The value of [`entry`], which `compute` never makes, so that a test
    /// can tell whether it was used.
    fn cached() -> Value {
        Value("from the cache".into(), 0)
    }

    /// An entry of [`cached`].
    fn entry(fingerprint: Option<Fingerprint>, hash: u128) -> Entry<Value> {
        let mut encoder = Encoder::default();
        cached().encode(&mut encoder);
        let bytes = encoder.into_bytes();
        let range = 0..bytes.len();
        Entry { stamp: Stamp { fingerprint, hash }, value: Encoded::new(Arc::new(bytes), range) }
    }

    /// The value of the file at `path`, computed from its contents.
    fn lookup(cache: &Cache<Value>, path: &Path, entry: Option<Entry<Value>>) -> Lookup<Value> {
        cache.lookup(&OsFileSystem, path, entry, |source| Value(source.to_string(), 1))
    }

    const SOURCE: &str = "import './a';\n";

    #[test]
    fn off_the_disk_a_file_is_read_and_its_contents_compared() {
        let (_dir, path, fingerprint) = file(SOURCE);
        // The disk's files, but not on the disk, as a commit's aren't.
        let off_disk = Overlay::default();
        let compute = |source: &str| Value(source.to_string(), 1);
        let cache = cache_written(fingerprint, RACY_WINDOW + Duration::from_millis(1));

        // On the disk, the fingerprint alone would be trusted.
        let stale = Some(entry(Some(fingerprint), 0));
        let found = cache.lookup(&off_disk, &path, stale, compute);
        assert_eq!(found.origin, Origin::Computed);
        assert_eq!(found.stamp.unwrap().fingerprint, None);

        let same = Some(entry(Some(fingerprint), xxh3_128(SOURCE.as_bytes())));
        let found = cache.lookup(&off_disk, &path, same, compute);
        assert_eq!(found.origin, Origin::Verified);
        assert_eq!(found.value.unwrap(), cached());
    }

    #[test]
    fn an_unchanged_fingerprint_is_trusted_once_it_is_old_enough() {
        let (_dir, path, fingerprint) = file(SOURCE);
        // The hash doesn't match, so only the fingerprint can make the entry be used.
        let stale = || Some(entry(Some(fingerprint), 0));

        let cache = cache_written(fingerprint, RACY_WINDOW + Duration::from_millis(1));
        let found = lookup(&cache, &path, stale());
        assert_eq!(found.origin, Origin::Unchanged);
        assert_eq!(found.value.unwrap(), cached());
        assert_eq!(found.stamp, Some(stale().unwrap().stamp));

        // A file modified within the window of the run that recorded it could have
        // changed again without its fingerprint changing, so it is read.
        for after in [Duration::ZERO, RACY_WINDOW] {
            let cache = cache_written(fingerprint, after);
            let found = lookup(&cache, &path, stale());
            assert_eq!(found.origin, Origin::Computed);
            assert_eq!(found.value.unwrap(), Value(SOURCE.into(), 1));
        }
    }

    #[test]
    fn unchanged_contents_are_reused_without_computing() {
        let (_dir, path, fingerprint) = file(SOURCE);
        let hash = xxh3_128(SOURCE.as_bytes());
        let old = Fingerprint { len: 1, ..fingerprint };
        let cache = cache_written(fingerprint, Duration::from_secs(60));

        // The fingerprint changed, or there was none, but the contents didn't.
        for recorded in [Some(old), None] {
            let found = lookup(&cache, &path, Some(entry(recorded, hash)));
            assert_eq!(found.origin, Origin::Verified);
            assert_eq!(found.value.unwrap(), cached());
            // The new stamp has the new fingerprint.
            assert_eq!(found.stamp, Some(Stamp { fingerprint: Some(fingerprint), hash }));
        }
    }

    #[test]
    fn changed_or_new_files_are_computed() {
        let (_dir, path, fingerprint) = file(SOURCE);
        let hash = xxh3_128(SOURCE.as_bytes());
        let cache = cache_written(fingerprint, Duration::from_secs(60));
        let old = Fingerprint { len: 1, ..fingerprint };
        for entry in [Some(entry(Some(old), hash ^ 1)), None] {
            let found = lookup(&cache, &path, entry);
            assert_eq!(found.origin, Origin::Computed);
            assert_eq!(found.value.unwrap(), Value(SOURCE.into(), 1));
            assert_eq!(found.stamp, Some(Stamp { fingerprint: Some(fingerprint), hash }));
        }
    }

    #[test]
    fn entries_whose_values_do_not_decode_are_computed() {
        let (_dir, path, fingerprint) = file(SOURCE);
        let hash = xxh3_128(SOURCE.as_bytes());
        let cache = cache_written(fingerprint, Duration::from_secs(60));
        let invalid = Arc::new(vec![1, 2, 3]);
        for recorded in [Some(fingerprint), None] {
            let value = Encoded::new(invalid.clone(), 0..invalid.len());
            let entry = Entry { stamp: Stamp { fingerprint: recorded, hash }, value };
            let found = lookup(&cache, &path, Some(entry));
            assert_eq!(found.origin, Origin::Computed);
            assert_eq!(found.value.unwrap(), Value(SOURCE.into(), 1));
        }
    }

    #[test]
    fn files_that_cannot_be_read_have_no_stamp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.ts");
        let cache =
            cache_written(Fingerprint { len: 0, modified: 0, changed: 0, inode: 0 }, RACY_WINDOW);
        let found = lookup(&cache, &path, Some(entry(None, 0)));
        assert_eq!(found.value.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!((found.stamp, found.origin), (None, Origin::Computed));
    }

    #[test]
    fn saves_and_loads_the_cache() {
        let dir = TempDir::new().unwrap();
        let settings = settings(dir.path());
        assert!(Cache::<Value>::load(settings.clone(), "inputs").is_empty());

        let stamp = Stamp { fingerprint: None, hash: 1 };
        let value = cached();
        let started = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let cache = Cache::<Value>::load(settings.clone(), "inputs");
        cache.save(started, [("a.ts", &stamp, &value)].into_iter()).unwrap();
        assert_eq!(
            fs::read_to_string(settings.dir.join(".gitignore")).unwrap(),
            "# Created by zenith.\n*\n"
        );

        let mut cache = Cache::<Value>::load(settings.clone(), "inputs");
        assert_eq!(cache.written_at, 10_000_000_000);
        assert_eq!(cache.len(), 1);
        let entry = cache.take("a.ts").unwrap();
        assert_eq!((entry.stamp, entry.value.decode()), (stamp, Some(value)));
        assert!(cache.take("a.ts").is_none());
        assert_eq!(cache.len(), 0);

        // Another version, other inputs, or a corrupt file, is an empty cache.
        let other = CacheSettings { version: "1.0.1".into(), ..settings.clone() };
        assert!(Cache::<Value>::load(other, "inputs").is_empty());
        assert!(Cache::<Value>::load(settings.clone(), "other inputs").is_empty());
        fs::write(settings.path::<Value>(), b"corrupt").unwrap();
        assert!(Cache::<Value>::load(settings, "inputs").is_empty());
    }
}
