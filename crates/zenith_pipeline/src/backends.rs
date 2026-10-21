//! The backends of the enabled languages, which the stages run.

use std::{fmt, path::Path};

use zenith_config::Config;
use zenith_lang::{Backend, BackendSettings, ImportResolver, ResolveFailure, SourceParser};

use crate::SourceFile;

/// The backend of each enabled language, in the order of
/// [`LanguagesConfig::enabled`](zenith_config::LanguagesConfig::enabled), so
/// that [`SourceFile::language`] picks a file's.
pub struct Backends(Vec<Box<dyn Backend>>);

impl Backends {
    /// The backends of the languages that `config` enables, set up for the
    /// project at `root`, which must be canonical.
    pub fn new(config: &Config, root: &Path) -> Self {
        let settings =
            BackendSettings { root, record_exports: config.needs_graph_through_reexports() };
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
