//! The [`BuildGraph`] stage.

use std::mem;

use zenith_graph::{GraphBuilder, NodeHandle};
use zenith_resolve::Resolved;

use crate::{FileSuppressions, PipelineError, Session, Stage, StageKind, UnresolvedImport};

/// Builds the import graph from the parsed files and their resolved imports,
/// which it takes out of the session. Imports that don't resolve are left out,
/// and recorded in the session, which also keeps the files' suppressions.
pub struct BuildGraph;

impl Stage for BuildGraph {
    fn kind(&self) -> StageKind {
        StageKind::Graph
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        let parsed = mem::take(&mut session.parsed);
        let resolved = mem::take(&mut session.resolved);
        let stats = &mut session.stats;

        // The builder sorts what it is given, so the order of the files
        // doesn't matter.
        let mut builder = GraphBuilder::new();
        for ((file, parsed), resolved) in session.files.iter().zip(parsed).zip(resolved) {
            let handle = builder.add_file(&file.name)?;
            builder.set_directives(handle, parsed.directives);
            stats.non_literal_imports += parsed.non_literal_imports.len();
            if !parsed.suppressions.is_empty() {
                // The files are sorted, so the suppressions are too.
                let suppressions = parsed.suppressions;
                session
                    .suppressions
                    .push(FileSuppressions { file: file.name.clone(), suppressions });
            }

            for (import, resolved) in parsed.imports.into_iter().zip(resolved) {
                let target: NodeHandle = match resolved {
                    Resolved::Internal(target) => builder.add_file(&target)?.into(),
                    Resolved::External { package, found } => {
                        stats.missing_packages += usize::from(!found);
                        builder.add_external(&format!("npm:{package}"))?
                    }
                    Resolved::Builtin(name) => builder.add_builtin(&name)?,
                    Resolved::Unresolved(reason) => {
                        let file = file.name.clone();
                        session.unresolved.push(UnresolvedImport { file, import, reason: *reason });
                        continue;
                    }
                    Resolved::Ignored(_) => continue,
                };
                builder.add_import(handle, target, import);
            }
        }

        // Sorting is stable, so each file's imports stay in source order.
        session.unresolved.sort_by(|a, b| a.file.cmp(&b.file));
        session.graph = builder.finish();
        Ok(())
    }
}
