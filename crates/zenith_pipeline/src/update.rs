//! Updating a session as the contents of its source files change: which
//! changes allow it, and how the stages that go file by file run on the
//! changed files alone, so that they need not know of it.

use std::{mem, path::Path};

use rustc_hash::{FxHashMap, FxHashSet};
use zenith_cache::CacheSettings;
use zenith_lang::{ParsedFile, Resolved};
use zenith_utils::path::relative_posix;

use crate::{FileError, ResolvedImport, Session, SourceFile};

/// What a session holds of every file, set aside while the stages that go
/// file by file run on the changed files alone.
pub(crate) struct Whole {
    /// Every source file.
    files: Vec<SourceFile>,

    /// What the parser found in each file.
    parsed: Vec<ParsedFile>,

    /// What the imports of each file resolved to.
    resolved: Vec<Vec<ResolvedImport>>,

    /// The files that could not be read or parsed.
    file_errors: Vec<FileError>,

    /// The parse cache, which is of every file.
    cache: Option<CacheSettings>,

    /// The changed files, by their indices in `files`.
    changed: Vec<u32>,
}

impl Session {
    /// The source files at `paths`, whose contents have changed since the
    /// stages ran, by their indices in [`Session::files`], sorted, for
    /// [`Pipeline::update`](crate::Pipeline::update). `None` if a change at
    /// one of the paths may change more than what those files import, so
    /// that the session must be built anew: if it is not one of the source
    /// files, or no longer exists, as a new file, a manifest or the
    /// configuration file is not.
    ///
    /// Changes that can't change what the stages find are left out: those to
    /// the parse cache, which the session writes itself, and those in git's
    /// own directory, unless only the files that git tracks are scanned.
    pub fn changed_files(
        &self,
        paths: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> Option<Vec<u32>> {
        let indices = file_indices(&self.files);
        let cache =
            (self.cache.as_ref()).map(|cache| self.file_system.adjust_canonicalisation(&cache.dir));
        let mut changed = Vec::new();
        for path in paths {
            let path = self.file_system.adjust_canonicalisation(path.as_ref());
            let in_git = || path.components().any(|component| component.as_os_str() == ".git");
            if cache.as_ref().is_some_and(|dir| path.starts_with(dir))
                || (!self.tracked_only && in_git())
            {
                continue;
            }
            let index = *indices.get(relative_posix(&self.root, &path)?.as_str())?;
            if !self.file_system.is_file(&path) {
                return None;
            }
            changed.push(index);
        }
        changed.sort_unstable();
        changed.dedup();
        Some(changed)
    }

    /// Whether the project is in the directory `dir`, a canonical path: its
    /// root, and its configuration file, if it was read from one.
    pub fn is_within(&self, dir: &Path) -> bool {
        let config = self.config.file.as_ref();
        let config = config.map(|file| self.file_system.adjust_canonicalisation(&file.path));
        self.root.starts_with(dir) && config.is_none_or(|config| config.starts_with(dir))
    }

    /// Set aside what the session holds of every file, so that it holds the
    /// files at `changed`, by their indices in [`Session::files`], alone, and
    /// nothing found in them yet, for the stages that go file by file to run
    /// on. The parse cache is set aside too, as it is of every file.
    pub(crate) fn focus(&mut self, changed: &[u32]) -> Whole {
        let files = changed.iter().map(|&index| self.files[index as usize].clone()).collect();
        Whole {
            files: mem::replace(&mut self.files, files),
            parsed: mem::take(&mut self.parsed),
            resolved: mem::take(&mut self.resolved),
            file_errors: mem::take(&mut self.file_errors),
            cache: self.cache.take(),
            changed: changed.to_vec(),
        }
    }

    /// Hold every file again, as `whole` was set aside, with what the stages
    /// found in the changed files in place of what they found in them
    /// before, and return whether it differs. A stage that didn't run found
    /// nothing, so what it found before stays.
    pub(crate) fn unfocus(&mut self, whole: Whole) -> bool {
        let Whole { files, mut parsed, mut resolved, mut file_errors, cache, changed } = whole;

        // 1. The problems of the changed files, in place of theirs, sorted by file.
        let names: FxHashSet<&str> = self.files.iter().map(|file| file.name.as_str()).collect();
        file_errors.retain(|error| !names.contains(error.file.as_str()));
        file_errors.append(&mut self.file_errors);
        file_errors.sort_by(|a, b| a.file.cmp(&b.file));

        // 2. What each file parsed and resolved to, in place of theirs, with the
        //    imports that resolved to a file naming it among every file.
        let indices = file_indices(&files);
        let mut found = mem::take(&mut self.resolved);
        found.iter_mut().flatten().for_each(|import| import.widen(&changed, &indices));
        let differs = put(&mut parsed, mem::take(&mut self.parsed), &changed)
            | put(&mut resolved, found, &changed);
        (self.files, self.parsed, self.resolved) = (files, parsed, resolved);
        (self.file_errors, self.cache) = (file_errors, cache);
        differs
    }
}

impl ResolvedImport {
    /// Name the file that the import resolved to among every file, whose
    /// `indices` are by name, as it was resolved among the files at
    /// `changed` alone, by their indices among every file.
    fn widen(&mut self, changed: &[u32], indices: &FxHashMap<&str, u32>) {
        let index = match self {
            Self::File(index) => changed[*index as usize],
            Self::Other(Resolved::Internal(name)) => match indices.get(name.as_str()) {
                Some(&index) => index,
                None => return,
            },
            Self::Other(_) => return,
        };
        *self = Self::File(index);
    }
}

/// The index of each of `files`, by its name.
pub(crate) fn file_indices(files: &[SourceFile]) -> FxHashMap<&str, u32> {
    (0..).zip(files).map(|(index, file)| (file.name.as_str(), index)).collect()
}

/// Put `found`, what a stage found in each of the files at `changed`, in
/// place of what `all` holds for them, and return whether any of it differs.
fn put<T: PartialEq>(all: &mut [T], found: Vec<T>, changed: &[u32]) -> bool {
    let mut differs = false;
    for (&index, found) in changed.iter().zip(found) {
        let held = &mut all[index as usize];
        differs |= *held != found;
        *held = found;
    }
    differs
}
