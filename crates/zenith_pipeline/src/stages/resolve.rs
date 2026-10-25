//! The [`Resolve`] stage.

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use zenith_lang::Resolved;

use crate::{PipelineError, ResolvedImport, Session, Stage, StageKind};

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
        let root = &session.root;
        // Most imports resolve to a source file, which is kept by its index, so that
        // their names don't pile up until the graph is built.
        let indices: FxHashMap<&str, u32> =
            (0..).zip(&session.files).map(|(index, file)| (file.name.as_str(), index)).collect();
        let resolved = session
            .files
            .par_iter()
            .zip(&session.parsed)
            .map(|(file, parsed)| {
                let resolver = &resolvers[usize::from(file.language)];
                let imports = parsed.imports.iter();
                let path = file.path(root);
                imports
                    .map(|import| match resolver.resolve_import(&path, import) {
                        Resolved::Internal(name) => match indices.get(name.as_str()) {
                            Some(index) => ResolvedImport::File(*index),
                            None => ResolvedImport::Other(Resolved::Internal(name)),
                        },
                        other => ResolvedImport::Other(other),
                    })
                    .collect()
            })
            .collect();

        session.resolved = resolved;
        Ok(())
    }
}
