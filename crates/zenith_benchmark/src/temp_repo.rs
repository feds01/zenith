//! Synthetic repositories written to temporary directories, for the
//! benchmarks that read from disk.

use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

use tempfile::TempDir;
use zenith_utils::path::adjust_canonicalisation;

use crate::SyntheticRepo;

/// The repositories that [`TempRepo::shared`] has written, by size.
static SHARED: Mutex<BTreeMap<usize, Arc<TempRepo>>> = Mutex::new(BTreeMap::new());

/// A [`SyntheticRepo`] written to a temporary directory, which is removed
/// when this is dropped.
#[derive(Debug)]
pub struct TempRepo {
    /// The repository that was written.
    pub repo: SyntheticRepo,

    /// The canonical path of the repository's root, as zenith expects it.
    pub root: PathBuf,

    /// The temporary directory, which lives as long as this does.
    _dir: TempDir,
}

impl TempRepo {
    /// Generate a repository with exactly `source_files` source files, and
    /// write it to a new temporary directory.
    ///
    /// # Panics
    ///
    /// If `source_files` is below [`crate::MIN_SOURCE_FILES`].
    pub fn new(source_files: usize) -> io::Result<Self> {
        let dir = tempfile::Builder::new().prefix("zenith-benchmark-").tempdir()?;
        let root = adjust_canonicalisation(dir.path());
        let repo = SyntheticRepo::new(source_files);
        repo.write(&root)?;
        Ok(Self { repo, root, _dir: dir })
    }

    /// The repository with `source_files` source files that this process
    /// shares, written the first time that it's asked for, so that the
    /// benchmarks of one binary only write each size once. Hold a
    /// [`TempRepo::remove_shared_on_drop`] guard in `main` to remove them.
    ///
    /// ```
    /// use zenith_benchmark::{TempRepo, default_walk_settings};
    ///
    /// // In `main`, around `divan::main()`.
    /// let _remove_shared = TempRepo::remove_shared_on_drop();
    ///
    /// // In each benchmark.
    /// let repo = TempRepo::shared(1_000);
    /// let found = zenith_workspace::walk(&repo.root, &default_walk_settings());
    /// ```
    ///
    /// # Panics
    ///
    /// If the repository can't be written, or `source_files` is below
    /// [`crate::MIN_SOURCE_FILES`].
    pub fn shared(source_files: usize) -> Arc<Self> {
        let mut shared = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        let repo = shared.entry(source_files).or_insert_with(|| {
            let repo = Self::new(source_files).unwrap_or_else(|error| {
                panic!("could not write a synthetic repository of {source_files} files: {error}")
            });
            Arc::new(repo)
        });
        Arc::clone(repo)
    }

    /// A guard that forgets the shared repositories when it's dropped, even
    /// if a benchmark panics, so that their temporary directories are removed
    /// once nothing else holds them. Statics are never dropped, so without it
    /// they would outlive the process.
    pub fn remove_shared_on_drop() -> RemoveShared {
        RemoveShared(())
    }
}

/// Removes the shared repositories when it's dropped. See
/// [`TempRepo::remove_shared_on_drop`].
#[must_use = "the shared repositories are removed when this is dropped"]
#[derive(Debug)]
pub struct RemoveShared(());

impl Drop for RemoveShared {
    fn drop(&mut self) {
        SHARED.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::Path};

    use zenith_lang_js::discover_packages;
    use zenith_utils::{fs::disk, path::relative_posix};
    use zenith_workspace::{WalkSettings, walk};

    use super::*;
    use crate::{FileKind, default_walk_settings};

    /// The paths of the repository's files of a kind, sorted.
    fn paths_of(repo: &SyntheticRepo, kind: FileKind) -> Vec<String> {
        repo.files().iter().filter(|file| file.kind == kind).map(|file| file.path.clone()).collect()
    }

    /// Paths found by the walk, relative to the root, sorted as strings.
    fn relative(root: &Path, paths: &[PathBuf]) -> Vec<String> {
        let mut paths: Vec<String> =
            paths.iter().map(|path| relative_posix(root, path).unwrap()).collect();
        paths.sort();
        paths
    }

    #[test]
    fn writes_every_file_byte_for_byte() {
        let temp = TempRepo::new(300).unwrap();
        for file in temp.repo.files() {
            let contents = fs::read_to_string(temp.root.join(&file.path)).unwrap();
            assert_eq!(contents, file.contents, "{}", file.path);
        }
    }

    /// The default settings with ignore files off, so that the developer's
    /// global git excludes can't change what the walk finds.
    fn without_ignore_files() -> WalkSettings {
        WalkSettings { respect_gitignore: false, ..default_walk_settings() }
    }

    #[test]
    fn walk_finds_the_source_files_and_manifests() {
        let temp = TempRepo::new(500).unwrap();
        let found = walk(&temp.root, &without_ignore_files());

        // Without ignore files, the `.gitignore`d generated code is found too.
        let gitignored: Vec<String> = paths_of(&temp.repo, FileKind::Skipped)
            .into_iter()
            .filter(|path| path.contains("/__generated__/"))
            .collect();
        assert!(!gitignored.is_empty());
        let mut expected = paths_of(&temp.repo, FileKind::Source);
        expected.extend(gitignored);
        expected.sort();

        assert_eq!(relative(&temp.root, &found.source_files), expected);
        assert_eq!(
            relative(&temp.root, &found.manifests),
            paths_of(&temp.repo, FileKind::Manifest)
        );
    }

    /// With the default settings, `.gitignore` hides the generated code. The
    /// developer's global git excludes could hide more, so this only checks
    /// that every file found is a source file.
    #[test]
    fn walk_respects_the_gitignore() {
        let temp = TempRepo::new(500).unwrap();
        let found = walk(&temp.root, &default_walk_settings());

        let sources: BTreeSet<String> =
            paths_of(&temp.repo, FileKind::Source).into_iter().collect();
        let found = relative(&temp.root, &found.source_files);
        assert!(!found.is_empty());
        for path in found {
            assert!(sources.contains(&path), "{path} isn't a source file");
        }
    }

    #[test]
    fn discovers_every_package() {
        let temp = TempRepo::new(1_000).unwrap();
        let found = walk(&temp.root, &without_ignore_files());
        let workspace = discover_packages(disk().as_ref(), &temp.root, &found.manifests);

        let discovered: Vec<(&str, &str)> = workspace
            .packages
            .iter()
            .map(|package| (package.name.as_str(), package.dir.as_str()))
            .collect();
        let expected: Vec<(&str, &str)> = temp
            .repo
            .packages()
            .iter()
            .map(|package| (package.name.as_str(), package.dir.as_str()))
            .collect();
        assert_eq!(discovered, expected);
        assert_eq!(workspace.root_package.map(|package| package.name).as_deref(), Some("acme"));
    }

    #[test]
    fn shared_repositories_are_written_once_and_removed() {
        let first = TempRepo::shared(200);
        let second = TempRepo::shared(200);
        assert!(Arc::ptr_eq(&first, &second));

        let root = first.root.clone();
        drop(TempRepo::remove_shared_on_drop());
        drop((first, second));
        assert!(!root.exists());
    }
}
