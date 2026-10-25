//! The [`Resolve`] stage.

use std::path::Path;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use zenith_lang::{ImportResolver, ParsedFile, Resolved};

use crate::{PipelineError, ResolvedImport, Session, SourceFile, Stage, StageKind};

/// Resolves the specifier of every parsed import with the backend of its
/// file's language, in parallel, sharing one resolver of each language and its
/// caches between the workers.
pub struct Resolve;

impl Stage for Resolve {
    fn kind(&self) -> StageKind {
        StageKind::Resolve
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        let resolvers = session.backends.resolvers()?;
        let indices = session.file_indices();
        let resolved = session
            .files
            .par_iter()
            .zip(&session.parsed)
            .map(|(file, parsed)| {
                let resolver = &*resolvers[usize::from(file.language)];
                resolve_imports(resolver, &indices, &session.root, file, parsed)
            })
            .collect();

        session.resolved = resolved;
        Ok(())
    }

    /// Resolves the imports of the changed files again. What the other files'
    /// imports resolve to stays: it depends on the files there are, and on
    /// the packages, not on what is in the files.
    fn update(&self, session: &mut Session, changed: &[u32]) -> Result<(), PipelineError> {
        let resolvers = session.backends.resolvers()?;
        let indices = session.file_indices();
        let resolved: Vec<_> = changed
            .par_iter()
            .map(|&index| {
                let (file, parsed) =
                    (&session.files[index as usize], &session.parsed[index as usize]);
                let resolver = &*resolvers[usize::from(file.language)];
                resolve_imports(resolver, &indices, &session.root, file, parsed)
            })
            .collect();
        for (&index, resolved) in changed.iter().zip(resolved) {
            session.resolved[index as usize] = resolved;
        }
        Ok(())
    }
}

/// What each import of `file`, which parsed to `parsed`, in the project at
/// `root`, resolves to with `resolver`. Most imports resolve to a source file,
/// which is kept by its index in `indices`, so that their names don't pile up
/// until the graph is built.
fn resolve_imports(
    resolver: &dyn ImportResolver,
    indices: &FxHashMap<&str, u32>,
    root: &Path,
    file: &SourceFile,
    parsed: &ParsedFile,
) -> Vec<ResolvedImport> {
    let path = file.path(root);
    (parsed.imports.iter())
        .map(|import| match resolver.resolve(&path, &import.specifier) {
            Resolved::Internal(name) => match indices.get(name.as_str()) {
                Some(index) => ResolvedImport::File(*index),
                None => ResolvedImport::Other(Resolved::Internal(name)),
            },
            other => ResolvedImport::Other(other),
        })
        .collect()
}
