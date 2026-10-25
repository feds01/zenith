//! The backends of the enabled languages, which the stages run.

use std::{fmt, path::Path, sync::Arc};

use zenith_config::Config;
use zenith_lang::{
    Backend, BackendSettings, ImportResolver, ParsedFile, ResolveFailure, SourceParser,
};
use zenith_utils::fs::FileSystem;

use crate::SourceFile;

/// The backend of each enabled language, in the order of
/// [`LanguagesConfig::enabled`](zenith_config::LanguagesConfig::enabled), so
/// that [`SourceFile::language`] picks a file's.
pub struct Backends(Vec<Box<dyn Backend>>);

impl Backends {
    /// The backends of the languages that `config` enables, set up for the
    /// project at `root` in `file_system`, which must be canonical there.
    /// With `record_exports`, they record what each file exports, for the
    /// graph through re-exports.
    pub fn new(
        config: &Config,
        root: &Path,
        file_system: &Arc<dyn FileSystem>,
        record_exports: bool,
    ) -> Self {
        let settings = BackendSettings { root, file_system, record_exports };
        Self(config.languages.iter().map(|(_, options)| options.backend(settings)).collect())
    }

    /// The backend of the language of `file`.
    pub fn of(&self, file: &SourceFile) -> &dyn Backend {
        self.get(usize::from(file.language))
    }

    /// The backend of the enabled language at `position`, as
    /// [`LanguagesConfig::language_of`](zenith_config::LanguagesConfig::language_of)
    /// gives it.
    pub fn get(&self, position: usize) -> &dyn Backend {
        self.0[position].as_ref()
    }

    /// Each backend, in order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Backend>> {
        self.0.iter_mut()
    }

    /// The names of the manifest files of every language.
    pub fn manifests(&self) -> Vec<&'static str> {
        let mut manifests: Vec<_> =
            self.0.iter().flat_map(|backend| backend.manifests()).copied().collect();
        manifests.sort_unstable();
        manifests.dedup();
        manifests
    }

    /// A parser of each language, for one thread, in order.
    pub fn parsers(&self) -> Vec<Box<dyn SourceParser>> {
        self.0.iter().map(|backend| backend.parser()).collect()
    }

    /// Let the backend of each language [link](Backend::link) its files of
    /// `files`, each with what it `parsed` to, in the same order, in the
    /// project at `root`.
    pub fn link(&mut self, root: &Path, files: &[SourceFile], parsed: &mut [ParsedFile]) {
        for (language, backend) in self.0.iter_mut().enumerate() {
            let mut files = (files.iter().zip(parsed.iter_mut()))
                .filter(|(file, _)| usize::from(file.language) == language)
                .map(|(file, parsed)| (file.path(root), parsed));
            backend.link(&mut files);
        }
    }

    /// A resolver of each language, in order.
    pub fn resolvers(&self) -> Result<Vec<Box<dyn ImportResolver + '_>>, ResolveFailure> {
        self.0.iter().map(|backend| backend.resolver()).collect()
    }
}

impl fmt::Debug for Backends {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Backends").field(&self.0.len()).finish()
    }
}
