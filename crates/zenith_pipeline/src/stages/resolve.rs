//! The [`Resolve`] stage.

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use zenith_resolve::Resolved;

use crate::{PipelineError, ResolvedImport, Session, Stage, StageKind};

/// Resolves the specifier of every parsed import with `zenith_resolve`, in
/// parallel, sharing one resolver and its caches between the workers.
pub struct Resolve;

impl Stage for Resolve {
    fn kind(&self) -> StageKind {
        StageKind::Resolve
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        let resolver = session.resolver()?;
        // Most imports resolve to a source file, which is kept by its index, so that
        // their names don't pile up until the graph is built.
        let indices: FxHashMap<&str, u32> =
            (0..).zip(&session.files).map(|(index, file)| (file.name.as_str(), index)).collect();
        let resolved = session
            .files
            .par_iter()
            .zip(&session.parsed)
            .map(|(file, parsed)| {
                let imports = parsed.imports.iter();
                imports
                    .map(|import| match resolver.resolve(&file.path, &import.specifier) {
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
