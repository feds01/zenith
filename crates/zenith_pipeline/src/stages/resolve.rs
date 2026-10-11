//! The [`Resolve`] stage.

use rayon::prelude::*;

use crate::{PipelineError, Session, Stage, StageKind};

/// Resolves the specifier of every parsed import with `zenith_resolve`, in
/// parallel, sharing one resolver and its caches between the workers.
pub struct Resolve;

impl Stage for Resolve {
    fn kind(&self) -> StageKind {
        StageKind::Resolve
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        let resolver = session.resolver()?;
        let resolved = session
            .files
            .par_iter()
            .zip(&session.parsed)
            .map(|(file, parsed)| {
                let imports = parsed.imports.iter();
                imports.map(|import| resolver.resolve(&file.path, &import.specifier)).collect()
            })
            .collect();

        session.resolved = resolved;
        Ok(())
    }
}
