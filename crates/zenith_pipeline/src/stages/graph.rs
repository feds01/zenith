//! The [`BuildGraph`] stage.

use std::mem;

use zenith_graph::{Exports, GraphBuilder, ModuleExports, NodeHandle};
use zenith_lang::{FileSuppressions, Resolved};

use crate::{
    PipelineError, ResolvedImport, Session, SourceFile, Stage, StageKind, UnresolvedImport,
};

/// Builds the import graph from the parsed files and their resolved imports,
/// which it takes out of the session, or copies if the session is
/// [updatable](Session::updatable). Imports that don't resolve are left out,
/// and recorded in the session, which also keeps the files' suppressions.
/// If a contract sees through re-exports, it builds the graph through them too,
/// from what each file exports.
pub struct BuildGraph;

impl Stage for BuildGraph {
    fn kind(&self) -> StageKind {
        StageKind::Graph
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        let (parsed, resolved) = match session.updatable {
            true => (session.parsed.clone(), session.resolved.clone()),
            false => (mem::take(&mut session.parsed), mem::take(&mut session.resolved)),
        };
        let through_reexports = session.needs_graph_through_reexports();
        // What a graph that was built before found goes with it.
        session.unresolved.clear();
        session.suppressions.clear();
        let stats = &mut session.stats;
        (stats.non_literal_imports, stats.missing_packages) = (0, 0);
        let mut file_exports = Vec::new();

        // The builder sorts what it is given, so the order of the files
        // doesn't matter.
        let mut builder = GraphBuilder::new();
        builder.set_optional_extensions(session.config.languages.optional_extensions());
        // Every source file is a node, which the imports that resolved to it find
        // by its index.
        let handles = session
            .files
            .iter()
            .map(|file| builder.add_file(&file.name))
            .collect::<Result<Vec<_>, _>>()?;
        let files = session.files.iter().zip(&handles);
        for (((file, &handle), parsed), resolved) in files.zip(parsed).zip(resolved) {
            builder.set_directives(handle, parsed.directives);
            stats.non_literal_imports += parsed.non_literal_imports.len();
            if !parsed.suppressions.is_empty() {
                // The files are sorted, so the suppressions are too.
                let suppressions = parsed.suppressions;
                session
                    .suppressions
                    .push(FileSuppressions { file: file.name.clone(), suppressions });
            }

            let scheme = session.backends.of(file).external_scheme();
            let mut targets = Vec::new();
            for (import, resolved) in parsed.imports.into_iter().zip(resolved) {
                if through_reexports {
                    targets.push(target_name(&session.files, scheme, &resolved));
                }
                let target: NodeHandle = match resolved {
                    ResolvedImport::File(index) => handles[index as usize].into(),
                    ResolvedImport::Other(Resolved::Internal(target)) => {
                        builder.add_file(&target)?.into()
                    }
                    ResolvedImport::Other(Resolved::Package(dir)) => {
                        builder.add_package_import(handle, &dir, import)?;
                        continue;
                    }
                    ResolvedImport::Other(Resolved::External { package, found }) => {
                        stats.missing_packages += usize::from(!found);
                        builder.add_external(&format!("{scheme}:{package}"))?
                    }
                    ResolvedImport::Other(Resolved::Builtin(name)) => builder.add_builtin(&name)?,
                    ResolvedImport::Other(Resolved::Unresolved(reason)) => {
                        let file = file.name.clone();
                        session.unresolved.push(UnresolvedImport { file, import, reason: *reason });
                        continue;
                    }
                    ResolvedImport::Other(Resolved::Ignored(_)) => continue,
                };
                builder.add_import(handle, target, import);
            }
            if through_reexports {
                file_exports.push(FileExports {
                    file: &file.name,
                    exports: parsed.exports,
                    targets,
                });
            }
        }

        // Sorting is stable, so each file's imports stay in source order.
        session.unresolved.sort_by(|a, b| a.file.cmp(&b.file));
        let graph = builder.finish();

        // Through re-exports, each re-export's source is the node its import resolved
        // to, if it did.
        if through_reexports {
            let exports: ModuleExports = file_exports
                .into_iter()
                .filter_map(|FileExports { file, exports, targets }| {
                    let exports = exports.map(|import| {
                        let target = targets.get(import as usize)?.as_deref()?;
                        graph.id(target)
                    });
                    Some((graph.id(file)?, exports))
                })
                .collect();
            session.graph_through_reexports = Some(graph.through_reexports(&exports));
        }
        session.graph = graph;
        Ok(())
    }

    /// Builds the graph again, as the graph can't change, from what the
    /// session kept and the stages before it updated.
    fn update(&self, session: &mut Session, _changed: &[u32]) -> Result<(), PipelineError> {
        self.run(session)
    }
}

/// The name of the node that an import resolved to, given the session's
/// `files` and the `scheme` of external packages, or `None` if it isn't in the
/// graph.
fn target_name(files: &[SourceFile], scheme: &str, resolved: &ResolvedImport) -> Option<String> {
    match resolved {
        ResolvedImport::File(index) => Some(files[*index as usize].name.clone()),
        ResolvedImport::Other(Resolved::Internal(name) | Resolved::Package(name)) => {
            Some(name.clone())
        }
        ResolvedImport::Other(Resolved::External { package, .. }) => {
            Some(format!("{scheme}:{package}"))
        }
        ResolvedImport::Other(Resolved::Builtin(name)) => Some(name.clone()),
        ResolvedImport::Other(Resolved::Unresolved(_) | Resolved::Ignored(_)) => None,
    }
}

/// What a file exports, as the graph through re-exports needs it.
struct FileExports<'s> {
    /// The file's name.
    file: &'s str,

    /// What it exports, with each re-export's source as the index of its
    /// import.
    exports: Exports<u32>,

    /// The name of the node each of its imports resolved to, in order.
    targets: Vec<Option<String>>,
}
