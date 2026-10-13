//! The parse cache: what the parser found in each file on an earlier run,
//! so that files that haven't changed since are neither read nor parsed
//! again.
//!
//! The cache is one file, [`CACHE_FILE_NAME`] in the cache directory, which
//! the [`Parse`](crate::Parse) stage reads at its start and rewrites at its
//! end if anything changed. Each entry is keyed by the file's name, and is
//! stamped with the file's [`Fingerprint`], its size, times and inode, and
//! the hash of its contents. A file is reused:
//!
//! - **unread**, if its fingerprint is unchanged, and old enough to trust: its
//!   modification time is more than [`RACY_WINDOW`] before the run that
//!   recorded it started. A file changed within that window of being read, in
//!   the same tick of a coarse clock, could otherwise keep its fingerprint, as
//!   git's racy-clean entries can.
//! - **read but not parsed**, if its fingerprint changed or is too recent, but
//!   its contents hash the same, e.g. after a checkout or a `touch`.
//!
//! Anything else is parsed again. The cache never changes what a run
//! finds: a file whose stamp doesn't match is parsed as if there were no
//! cache, and a cache file that is missing, corrupt, or written by another
//! version of zenith or with other [`ParseOptions`] is ignored, and replaced
//! at the end of the run.
//!
//! Concurrent runs are safe: a run reads the whole file at once, and
//! replaces it by renaming a complete temporary file over it, so the file is
//! always one run's cache, whichever wrote last.

mod encoding;

use std::{
    fs::{self, File, Metadata},
    hash::{Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use log::debug;
use rustc_hash::FxHashMap;
use tempfile::NamedTempFile;
use xxhash_rust::xxh3::{Xxh3, xxh3_128};
use zenith_parse::{ParseOptions, ParsedFile, Parser};
use zenith_utils::fs::read_text_from;

use crate::cache::encoding::Rejected;

/// The name of the cache file in the cache directory.
pub const CACHE_FILE_NAME: &str = "parse.bin";

/// How long before a run starts a file must have last changed for its
/// fingerprint to be trusted on the next run. This covers the coarsest
/// modification times in common use, FAT's two seconds.
const RACY_WINDOW: Duration = Duration::from_secs(2);

/// Where to keep the parse cache, and the version of zenith that it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSettings {
    /// The directory of the cache. It is created when the cache is first
    /// written, with a `.gitignore` that ignores everything in it.
    pub dir: PathBuf,

    /// The version of zenith, which a cache file must have been written by
    /// to be used. What the parser finds may change between versions, so a
    /// new version starts from an empty cache.
    pub version: String,
}

impl CacheSettings {
    /// The path of the cache file.
    pub fn file(&self) -> PathBuf {
        self.dir.join(CACHE_FILE_NAME)
    }

    /// The key of a cache of files parsed with `options`: the hash of the
    /// version and the options, which a cache file must have been written
    /// with to be used. Hashing every option, rather than naming them, keeps
    /// a new one from being left out.
    fn key(&self, options: ParseOptions) -> u64 {
        let mut hasher = Xxh3::new();
        (self.version.as_str(), options).hash(&mut hasher);
        hasher.finish()
    }
}

/// What a file's metadata says about its contents. If it is unchanged, the
/// contents almost certainly are too.
///
/// The change time and inode are only known on Unix. On Windows, the
/// creation time stands in for the change time, and the inode is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fingerprint {
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
pub(crate) struct Stamp {
    /// The fingerprint of the file's metadata, taken before it was read, or
    /// `None` if it couldn't be taken.
    fingerprint: Option<Fingerprint>,

    /// The XXH3 hash of the file's contents.
    hash: u128,
}

/// A file's entry in the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// When the file was last seen.
    stamp: Stamp,

    /// What the parser found in it.
    parsed: ParsedFile,
}

/// Where a file's [`ParsedFile`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The cache, without reading the file, since its fingerprint was
    /// unchanged.
    Unchanged,

    /// The cache, after reading the file and finding that its contents were
    /// unchanged.
    Verified,

    /// The parser, since the file had no entry, or had changed.
    Parsed,
}

/// What a file parsed to, from the cache or the parser.
#[derive(Debug)]
pub(crate) struct Lookup {
    /// What the parser found in the file, or why it couldn't be read.
    pub(crate) parsed: io::Result<ParsedFile>,

    /// The file's stamp, for its entry in the next cache, or `None` if it
    /// couldn't be read.
    pub(crate) stamp: Option<Stamp>,

    /// Where `parsed` came from.
    pub(crate) origin: Origin,
}

/// The cache that an earlier run wrote, as the [`Parse`](crate::Parse)
/// stage takes it apart.
#[derive(Debug, Default)]
pub(crate) struct ParseCache {
    /// When the run that wrote the cache started, in nanoseconds since the
    /// Unix epoch.
    written_at: u64,

    /// The entries that haven't been taken yet, by the name of their file.
    entries: FxHashMap<String, Entry>,
}

impl ParseCache {
    /// Read the cache file of `settings`, of files parsed with `options`. A
    /// cache file that is missing, corrupt, or written by another version of
    /// zenith or with other options is an empty cache.
    pub(crate) fn load(settings: &CacheSettings, options: ParseOptions) -> Self {
        let path = settings.file();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                debug!("no parse cache at `{}`: {error}", path.display());
                return Self::default();
            }
        };
        match encoding::decode(&bytes, settings.key(options)) {
            Ok(decoded) => {
                let (written_at, entries) = (decoded.written_at, decoded.entries);
                debug!("read the parse cache of {} files", entries.len());
                Self { written_at, entries }
            }
            Err(Rejected::OtherKey) => {
                debug!("ignoring the parse cache of another version of zenith, or other options");
                Self::default()
            }
            Err(Rejected::Corrupt) => {
                debug!("ignoring the parse cache at `{}`, which is corrupt", path.display());
                Self::default()
            }
        }
    }

    /// The number of entries that haven't been taken.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Take the entry of the file `name` out of the cache.
    pub(crate) fn take(&mut self, name: &str) -> Option<Entry> {
        self.entries.remove(name)
    }

    /// Parse the file at `path` with `parser`, unless `entry`, its entry in
    /// this cache, shows that it is unchanged.
    pub(crate) fn parse(
        &self,
        parser: &mut Parser,
        path: &Path,
        mut entry: Option<Entry>,
    ) -> Lookup {
        // 1. A file with an entry is likely unchanged, so look at its metadata alone
        //    first.
        if entry.is_some() {
            let fingerprint = fs::metadata(path).ok().as_ref().and_then(Fingerprint::new);
            let unchanged = |entry: &mut Entry| {
                fingerprint.is_some_and(|fingerprint| {
                    entry.stamp.fingerprint == Some(fingerprint) && self.trusts(fingerprint)
                })
            };
            if let Some(Entry { stamp, parsed }) = entry.take_if(unchanged) {
                let stamp = Some(stamp);
                return Lookup { parsed: Ok(parsed), stamp, origin: Origin::Unchanged };
            }
        }

        // 2. Read the file, taking its fingerprint first, so that a change made while
        //    it is read makes it differ next time. Reuse the entry if the contents hash
        //    the same.
        let (source, fingerprint) = match read_fingerprinted(path) {
            Ok(read) => read,
            Err(error) => {
                return Lookup { parsed: Err(error), stamp: None, origin: Origin::Parsed };
            }
        };
        let stamp = Some(Stamp { fingerprint, hash: xxh3_128(source.as_bytes()) });
        if let Some(entry) = entry
            && Some(entry.stamp.hash) == stamp.map(|stamp| stamp.hash)
        {
            return Lookup { parsed: Ok(entry.parsed), stamp, origin: Origin::Verified };
        }

        // 3. Otherwise, parse it.
        let parsed = Ok(parser.parse_source(path, &source));
        Lookup { parsed, stamp, origin: Origin::Parsed }
    }

    /// Whether a file whose unchanged fingerprint is `fingerprint` can be
    /// trusted to be unchanged: whether it was last modified well before the
    /// run that recorded it started.
    fn trusts(&self, fingerprint: Fingerprint) -> bool {
        let window = RACY_WINDOW.as_nanos() as u64;
        fingerprint.modified.saturating_add(window) < self.written_at
    }
}

/// Read the text file at `path`, as [`read_text`] does, and the fingerprint
/// of its metadata, taken before it is read. Opening it once for both is
/// faster than looking it up twice.
fn read_fingerprinted(path: &Path) -> io::Result<(String, Option<Fingerprint>)> {
    let mut file = File::open(path)?;
    let fingerprint = file.metadata().ok().as_ref().and_then(Fingerprint::new);
    Ok((read_text_from(&mut file)?, fingerprint))
}

/// Write the cache of the run that started at `started`, which parsed with
/// `options`, with an entry for each of `entries`, replacing the cache file of
/// `settings`.
pub(crate) fn save<'a>(
    settings: &CacheSettings,
    options: ParseOptions,
    started: SystemTime,
    entries: impl ExactSizeIterator<Item = (&'a str, &'a Stamp, &'a ParsedFile)>,
) -> io::Result<()> {
    // 1. Create the directory, which git should ignore wherever it is.
    let dir = &settings.dir;
    fs::create_dir_all(dir)?;
    let gitignore = dir.join(".gitignore");
    if !gitignore.exists() {
        fs::write(gitignore, "# Created by zenith.\n*\n")?;
    }

    // 2. Write the whole file next to the cache file, then rename it over it, so
    //    that other runs read either the old cache or the new one.
    let written_at = nanos_since_epoch(started).unwrap_or(0);
    let bytes = encoding::encode(settings.key(options), written_at, entries);
    let mut file = NamedTempFile::new_in(dir)?;
    file.write_all(&bytes)?;
    file.persist(settings.file()).map_err(|error| error.error)?;
    Ok(())
}

/// The nanoseconds from the Unix epoch to `time`, or `None` if it is before
/// it.
fn nanos_since_epoch(time: SystemTime) -> Option<u64> {
    let since = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    u64::try_from(since.as_nanos()).ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;
    use thin_vec::thin_vec;

    use super::*;

    /// A file with `source` in a new temporary directory, and its
    /// fingerprint.
    fn file(source: &str) -> (TempDir, PathBuf, Fingerprint) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.ts");
        fs::write(&path, source).unwrap();
        let fingerprint = Fingerprint::new(&fs::metadata(&path).unwrap()).unwrap();
        (dir, path, fingerprint)
    }

    /// A cache written `after` the file with `fingerprint` was last
    /// modified.
    fn cache_written(fingerprint: Fingerprint, after: Duration) -> ParseCache {
        let written_at = fingerprint.modified + after.as_nanos() as u64;
        ParseCache { written_at, entries: FxHashMap::default() }
    }

    /// An entry that no parser would find in `import './a';`, so that a test
    /// can tell whether it was used.
    fn entry(fingerprint: Option<Fingerprint>, hash: u128) -> Entry {
        let parsed =
            ParsedFile { directives: thin_vec!["from the cache".into()], ..Default::default() };
        Entry { stamp: Stamp { fingerprint, hash }, parsed }
    }

    fn parse(cache: &ParseCache, path: &Path, entry: Option<Entry>) -> Lookup {
        cache.parse(&mut Parser::new(), path, entry)
    }

    const SOURCE: &str = "import './a';\n";

    #[test]
    fn an_unchanged_fingerprint_is_trusted_once_it_is_old_enough() {
        let (_dir, path, fingerprint) = file(SOURCE);
        // The hash doesn't match, so only the fingerprint can make the entry be used.
        let stale = || Some(entry(Some(fingerprint), 0));

        let cache = cache_written(fingerprint, RACY_WINDOW + Duration::from_millis(1));
        let lookup = parse(&cache, &path, stale());
        assert_eq!(lookup.origin, Origin::Unchanged);
        assert_eq!(lookup.parsed.unwrap(), stale().unwrap().parsed);
        assert_eq!(lookup.stamp, Some(stale().unwrap().stamp));

        // A file modified within the window of the run that recorded it could have
        // changed again without its fingerprint changing, so it is read.
        for after in [Duration::ZERO, RACY_WINDOW] {
            let cache = cache_written(fingerprint, after);
            let lookup = parse(&cache, &path, stale());
            assert_eq!(lookup.origin, Origin::Parsed);
            assert_eq!(lookup.parsed.unwrap().imports[0].specifier, "./a");
        }
    }

    #[test]
    fn unchanged_contents_are_reused_without_parsing() {
        let (_dir, path, fingerprint) = file(SOURCE);
        let hash = xxh3_128(SOURCE.as_bytes());
        let old = Fingerprint { len: 1, ..fingerprint };
        let cache = cache_written(fingerprint, Duration::from_secs(60));

        // The fingerprint changed, or there was none, but the contents didn't.
        for recorded in [Some(old), None] {
            let lookup = parse(&cache, &path, Some(entry(recorded, hash)));
            assert_eq!(lookup.origin, Origin::Verified);
            assert_eq!(lookup.parsed.unwrap(), entry(None, 0).parsed);
            // The new stamp has the new fingerprint.
            assert_eq!(lookup.stamp, Some(Stamp { fingerprint: Some(fingerprint), hash }));
        }
    }

    #[test]
    fn changed_or_new_files_are_parsed() {
        let (_dir, path, fingerprint) = file(SOURCE);
        let hash = xxh3_128(SOURCE.as_bytes());
        let cache = cache_written(fingerprint, Duration::from_secs(60));
        let old = Fingerprint { len: 1, ..fingerprint };
        for entry in [Some(entry(Some(old), hash ^ 1)), None] {
            let lookup = parse(&cache, &path, entry);
            assert_eq!(lookup.origin, Origin::Parsed);
            assert_eq!(lookup.parsed.unwrap(), Parser::new().parse_source(&path, SOURCE));
            assert_eq!(lookup.stamp, Some(Stamp { fingerprint: Some(fingerprint), hash }));
        }
    }

    #[test]
    fn files_that_cannot_be_read_have_no_stamp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.ts");
        let lookup = parse(&ParseCache::default(), &path, Some(entry(None, 0)));
        assert_eq!(lookup.parsed.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!((lookup.stamp, lookup.origin), (None, Origin::Parsed));
    }

    #[test]
    fn saves_and_loads_the_cache() {
        let dir = TempDir::new().unwrap();
        let settings = CacheSettings { dir: dir.path().join("cache"), version: "1.0.0".into() };
        assert!(ParseCache::load(&settings, ParseOptions::default()).entries.is_empty());

        let stamp = Stamp { fingerprint: None, hash: 1 };
        let parsed = entry(None, 0).parsed;
        let started = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        save(&settings, ParseOptions::default(), started, [("a.ts", &stamp, &parsed)].into_iter())
            .unwrap();
        assert_eq!(
            fs::read_to_string(settings.dir.join(".gitignore")).unwrap(),
            "# Created by zenith.\n*\n"
        );

        let mut cache = ParseCache::load(&settings, ParseOptions::default());
        assert_eq!(cache.written_at, 10_000_000_000);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.take("a.ts"), Some(Entry { stamp, parsed }));
        assert_eq!((cache.take("a.ts"), cache.len()), (None, 0));

        // Another version, other options, or a corrupt file, is an empty cache.
        let other = CacheSettings { version: "1.0.1".into(), ..settings.clone() };
        assert_eq!(ParseCache::load(&other, ParseOptions::default()).len(), 0);
        assert_eq!(ParseCache::load(&settings, ParseOptions { names: true }).len(), 0);
        fs::write(settings.file(), b"corrupt").unwrap();
        assert_eq!(ParseCache::load(&settings, ParseOptions::default()).len(), 0);
    }
}
