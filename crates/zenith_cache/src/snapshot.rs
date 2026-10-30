//! A value that depends on what is at some paths on the disk, kept between
//! runs while nothing there changes.
//!
//! A [`Snapshot`] keeps one [`Cached`] value, such as the files that a walk
//! found, with the paths that it depends on: those of whose contents it was
//! made, the directories whose entries it read and the files that it read,
//! each recorded with its [`Fingerprint`], and those at which only the kind
//! of what is there matters, recorded as a file or a directory. A path with
//! nothing at it is recorded as such. The value is reused while what is at
//! every path is as it was recorded, and each fingerprint is old enough to
//! trust, as the [cache](crate::Cache)'s are: its modification time is more
//! than [`RACY_WINDOW`] before the run that recorded it started. Adding,
//! removing or renaming an entry of a directory changes the directory's
//! modification time, so a directory with the same fingerprint has the same
//! entries.
//!
//! A path with a fingerprint that changed while the value was made has a
//! modification time after the run started, so the value is never trusted
//! with it, however late the fingerprint is taken. A path recorded as missing,
//! or by its kind, has no such time, so the paths that the value depends on
//! before it is made are [observed](Snapshot::observe) then, and the value is
//! only saved if they are still the same.

use std::{
    fs,
    hash::Hash,
    io,
    marker::PhantomData,
    path::{Path, PathBuf},
    time::SystemTime,
};

use log::debug;
use rayon::prelude::*;

use crate::{
    CacheSettings, Cached, Encoder, Fingerprint,
    encoding::{self, Rejected},
    nanos_since_epoch,
};

/// The snapshot of a `T` in a cache directory.
#[derive(Debug)]
pub struct Snapshot<T> {
    /// Where the snapshot is kept.
    settings: CacheSettings,

    /// The hash of the version and the inputs of the value, which the file
    /// must have been written with.
    key: u64,

    /// The type of the value.
    value: PhantomData<fn() -> T>,
}

impl<T: Cached> Snapshot<T> {
    /// The snapshot in the cache of `settings`, of a value computed with
    /// `inputs`, which are hashed into its key, as a [`Cache`](crate::Cache)'s
    /// are.
    pub fn new(settings: CacheSettings, inputs: impl Hash) -> Self {
        let key = settings.key(inputs);
        Self { settings, key, value: PhantomData }
    }

    /// The value last saved, if every path that it depends on is unchanged
    /// since, and was last changed well before it was saved. `None` if there
    /// is no such value, or its file is missing, corrupt, or written by
    /// another version of zenith or with other inputs.
    pub fn load(&self) -> Option<T> {
        let decoded = self.settings.read::<T, _>(|bytes| decode::<T>(&bytes, self.key))?;
        let trusted = |recorded: &State| match recorded {
            State::Fingerprint(fingerprint) => fingerprint.trusted(decoded.written_at),
            State::Missing | State::File | State::Directory => true,
        };
        let changed = (decoded.paths.par_iter()).find_any(|(path, recorded)| {
            let contents = matches!(recorded, State::Fingerprint(_));
            !trusted(recorded) || State::of(path, contents).ok() != Some(*recorded)
        });
        let path = self.settings.path::<T>();
        if let Some((changed, _)) = changed {
            debug!(
                "the snapshot at `{}` is out of date: `{}` changed",
                path.display(),
                changed.display()
            );
            return None;
        }
        debug!("reusing the snapshot at `{}`", path.display());
        Some(decoded.value)
    }

    /// What is at `contents` and `kinds` now, as [`Snapshot::save`] records
    /// it, taken before a value is made from them so that saving it can tell
    /// whether they changed meanwhile.
    pub fn observe(&self, contents: &[PathBuf], kinds: &[PathBuf]) -> Observed {
        Observed(observe(contents, kinds))
    }

    /// Save `value`, which the run that started at `started` made from the
    /// contents of what is at `contents`, and the kinds of what is at
    /// `kinds`, and from what was at the paths of `before`, replacing the
    /// snapshot. What is at the paths is recorded now. Nothing is saved if
    /// what is at one of the paths of `before` changed since, if a path isn't
    /// UTF-8, or if what is at it can't be told.
    ///
    /// A path recorded with a fingerprint is only trusted if it changed long
    /// enough before the run, which a path recorded as missing, or by its
    /// kind, has no time to tell; so those that a value depends on before it
    /// is made belong in `before`, and those found while it is made must be
    /// in a directory recorded with its fingerprint.
    pub fn save(
        &self,
        started: SystemTime,
        value: &T,
        before: &Observed,
        contents: &[PathBuf],
        kinds: &[PathBuf],
    ) -> io::Result<()> {
        let Some(before) = &before.0 else {
            debug!("not saving a snapshot that depends on paths that can't be told");
            return Ok(());
        };
        let unchanged: Option<Vec<_>> = (before.par_iter())
            .map(|(path, before)| {
                let state = State::of(path, matches!(before, State::Fingerprint(_))).ok()?;
                (state == *before).then(|| (path.clone(), state))
            })
            .collect();
        let Some(mut states) = unchanged else {
            debug!("not saving a snapshot of paths that changed while it was made");
            return Ok(());
        };
        let Some(after) = observe(contents, kinds) else {
            debug!("not saving a snapshot that depends on paths that can't be told");
            return Ok(());
        };
        states.extend(after);
        let states: Option<Vec<_>> =
            states.iter().map(|(path, state)| Some((path.to_str()?, *state))).collect();
        let Some(states) = states else {
            debug!("not saving a snapshot that depends on paths that aren't UTF-8");
            return Ok(());
        };
        let written_at = nanos_since_epoch(started).unwrap_or(0);
        self.settings.write::<T>(&encode(self.key, written_at, &states, value))
    }
}

/// What was at some paths when a snapshot's value was about to be made, or
/// `None` if what was at one of them couldn't be told.
#[derive(Debug)]
pub struct Observed(Option<Vec<(PathBuf, State)>>);

/// What is at `contents`, by fingerprint, and at `kinds`, by kind, or
/// `None` if what is at one of them can't be told.
fn observe(contents: &[PathBuf], kinds: &[PathBuf]) -> Option<Vec<(PathBuf, State)>> {
    let paths = (contents.iter().map(|path| (path, true)))
        .chain(kinds.iter().map(|path| (path, false)))
        .collect::<Vec<_>>();
    paths
        .par_iter()
        .map(|(path, contents)| Some((path.to_path_buf(), State::of(path, *contents).ok()?)))
        .collect()
}

/// A snapshot's file, as [`encode`] writes it: when the run that
/// wrote it started, the paths that its value depends on, each with what was
/// there, and the value.
#[derive(Debug)]
struct Decoded<T> {
    /// When the run that wrote the file started, in nanoseconds since the
    /// Unix epoch.
    written_at: u64,

    /// The paths, each with what was there.
    paths: Vec<(PathBuf, State)>,

    /// The value.
    value: T,
}

/// Encode the file of a snapshot with `key`, of the run that started at
/// `written_at`: the `paths` that `value` depends on, each with what was
/// there, then `value`. A snapshot's file has the header of a cache file,
/// and its body is the time, the list of paths, then the value. What was at
/// a path is a byte, 0 for nothing, 1 for a file, 2 for a directory, or 3
/// followed by a fingerprint.
fn encode<T: Cached>(key: u64, written_at: u64, paths: &[(&str, State)], value: &T) -> Vec<u8> {
    let mut body = Encoder::default();
    body.u64(written_at);
    body.length(paths.len());
    for (path, state) in paths {
        body.str(path);
        match state {
            State::Missing => body.u8(0),
            State::File => body.u8(1),
            State::Directory => body.u8(2),
            State::Fingerprint(fingerprint) => {
                body.u8(3);
                body.fingerprint(fingerprint);
            }
        }
    }
    value.encode(&mut body);
    encoding::wrap(key, body)
}

/// Decode the file of a snapshot written with `key`.
fn decode<T: Cached>(bytes: &[u8], key: u64) -> Result<Decoded<T>, Rejected> {
    let mut decoder = encoding::unwrap(bytes, key)?;
    let written_at = decoder.u64().ok_or(Rejected::Corrupt)?;
    let paths = decoder
        .list(|decoder| {
            let path = PathBuf::from(decoder.str()?);
            let state = match decoder.u8()? {
                0 => State::Missing,
                1 => State::File,
                2 => State::Directory,
                3 => State::Fingerprint(decoder.fingerprint()?),
                _ => return None,
            };
            Some((path, state))
        })
        .ok_or(Rejected::Corrupt)?;
    let value = T::decode(&mut decoder).ok_or(Rejected::Corrupt)?;
    if !decoder.is_empty() {
        return Err(Rejected::Corrupt);
    }
    Ok(Decoded { written_at, paths, value })
}

/// What is at a path that a snapshot depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// Nothing.
    Missing,

    /// A file, whose contents don't matter.
    File,

    /// A directory, whose entries don't matter.
    Directory,

    /// A file or a directory with this fingerprint, whose contents or
    /// entries matter.
    Fingerprint(Fingerprint),
}

impl State {
    /// What is at `path`, following links: its fingerprint if its
    /// `contents` matter, and otherwise its kind. An error if it can't be
    /// told, or its modification time can't be.
    fn of(path: &Path, contents: bool) -> io::Result<Self> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::Missing),
            Err(error) => return Err(error),
        };
        Ok(match contents {
            true => Self::Fingerprint(Fingerprint::new(&metadata).ok_or(io::ErrorKind::Other)?),
            false if metadata.is_dir() => Self::Directory,
            false => Self::File,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        RACY_WINDOW,
        encoding::tests::{KEY, Value, stamp},
    };

    /// A project with a directory `src` holding `a.ts`, and a `.git`
    /// directory, and the settings of a cache in it.
    fn project() -> (TempDir, CacheSettings) {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a.ts"), "").unwrap();
        let settings = CacheSettings { dir: dir.path().join("cache"), version: "1.0.0".into() };
        (dir, settings)
    }

    /// Save `value` in `snapshot` of the project in `dir`, as made from the
    /// contents of `src` and `src/a.ts`, found while it was made, and from
    /// those of `.gitignore`, which isn't there, and whatever is at `.git`,
    /// observed before, by a run that started `started`.
    fn save(snapshot: &Snapshot<Value>, dir: &Path, started: SystemTime) {
        let before = snapshot.observe(&[dir.join(".gitignore")], &[dir.join(".git")]);
        let contents = [dir.join("src"), dir.join("src/a.ts")];
        snapshot.save(started, &Value("found".into(), 1), &before, &contents, &[]).unwrap();
    }

    /// A time well after every file of a test was last changed, so that their
    /// fingerprints are trusted.
    fn later() -> SystemTime {
        SystemTime::now() + RACY_WINDOW + Duration::from_secs(1)
    }

    #[test]
    fn decodes_the_snapshots_it_encodes() {
        let fingerprint = State::Fingerprint(stamp().fingerprint.unwrap());
        let paths = [
            ("/repo/src", fingerprint),
            ("/repo/.gitignore", State::Missing),
            ("/repo/.git", State::Directory),
            ("/.git", State::File),
        ];
        let bytes = encode(KEY, 42, &paths, &Value("é".into(), 1));
        let decoded = decode::<Value>(&bytes, KEY).unwrap();
        assert_eq!(decoded.written_at, 42);
        let expected = paths.map(|(path, state)| (PathBuf::from(path), state));
        assert_eq!(decoded.paths, expected);
        assert_eq!(decoded.value, Value("é".into(), 1));

        assert_eq!(decode::<Value>(&bytes, KEY + 1).unwrap_err(), Rejected::OtherKey);
        for len in 0..bytes.len() {
            assert!(decode::<Value>(&bytes[..len], KEY).is_err(), "truncated to {len}");
        }
    }

    #[test]
    fn a_snapshot_is_reused_while_its_paths_are_unchanged() {
        let (dir, settings) = project();
        let snapshot = Snapshot::<Value>::new(settings.clone(), "inputs");
        assert_eq!(snapshot.load(), None);

        save(&snapshot, dir.path(), later());
        assert_eq!(snapshot.load(), Some(Value("found".into(), 1)));

        // What is in a directory whose kind is all that matters doesn't.
        fs::write(dir.path().join(".git/index.lock"), "").unwrap();
        assert_eq!(snapshot.load(), Some(Value("found".into(), 1)));

        // Other inputs have their own snapshot.
        assert_eq!(Snapshot::<Value>::new(settings, "other inputs").load(), None);
    }

    #[test]
    fn a_change_at_any_path_invalidates_it() {
        let changes: [fn(&Path); 4] = [
            |root| fs::write(root.join("src/b.ts"), "").unwrap(),
            |root| fs::write(root.join("src/a.ts"), "changed").unwrap(),
            |root| fs::write(root.join(".gitignore"), "").unwrap(),
            |root| {
                fs::remove_dir(root.join(".git")).unwrap();
                fs::write(root.join(".git"), "gitdir: elsewhere").unwrap();
            },
        ];
        for change in changes {
            let (dir, settings) = project();
            let snapshot = Snapshot::<Value>::new(settings, "inputs");
            save(&snapshot, dir.path(), later());
            change(dir.path());
            assert_eq!(snapshot.load(), None);
        }
    }

    #[test]
    fn paths_observed_before_that_change_before_saving_save_nothing() {
        let (dir, settings) = project();
        let snapshot = Snapshot::<Value>::new(settings, "inputs");
        let before = snapshot.observe(&[dir.path().join(".gitignore")], &[]);
        fs::write(dir.path().join(".gitignore"), "").unwrap();
        snapshot.save(later(), &Value("found".into(), 1), &before, &[], &[]).unwrap();
        assert_eq!(snapshot.load(), None);
        assert!(!snapshot.settings.path::<Value>().exists());
    }

    #[test]
    fn paths_changed_too_close_to_the_run_are_not_trusted() {
        let (dir, settings) = project();
        let snapshot = Snapshot::<Value>::new(settings, "inputs");
        save(&snapshot, dir.path(), SystemTime::now());
        assert_eq!(snapshot.load(), None);
    }
}
