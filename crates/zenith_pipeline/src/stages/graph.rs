//! The [`BuildGraph`] stage.

use zenith_graph::{Exports, FileHandle, GraphBuilder, ModuleExports, NodeHandle};
use zenith_lang::{FileSuppressions, Resolved};

use crate::{PipelineError, ResolvedImport, Session, Stage, StageKind, UnresolvedImport};

/// Builds the import graph from the parsed files and their resolved imports,
/// which it leaves in the session, so that it can build the graph again when
/// they change. Imports that don't resolve are left out, and recorded in the
/// session, which also keeps the files' suppressions.
/// If a contract sees through re-exports, it builds the graph through them too,
/// from what each file exports.
pub struct BuildGraph;

impl Stage for BuildGraph {
    fn kind(&self) -> StageKind {
        StageKind::Graph
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        let (parsed, resolved) = (session.parsed.clone(), session.resolved.clone());
        let through_reexports = session.needs_graph_through_reexports();
        let (mut unresolved, mut suppressions, mut file_exports) =
            (Vec::new(), Vec::new(), Vec::new());
        let (mut non_literal_imports, mut missing_packages) = (0, 0);

        // The builder sorts what it is given, so the order of the files
        // doesn't matter.
        let mut builder = GraphBuilder::new();
        builder.set_optional_extensions(session.config.languages.optional_extensions());
        builder.set_directory_modules(session.config.languages.directory_modules());
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
            non_literal_imports += parsed.non_literal_imports.len();
            if !parsed.suppressions.is_empty() {
                // The files are sorted, so the suppressions are too. Each name is the id
                // of the contract it names, as a member's files may name its contracts
                // by their own ids.
                let mut file_suppressions = parsed.suppressions;
                for name in file_suppressions.iter_mut().flat_map(|each| &mut each.value.contracts)
                {
                    if let Some(contract) = session.config.contract_named(name, &file.name) {
                        name.clone_from(&contract.id);
                    }
                }
                let file = file.name.clone();
                suppressions.push(FileSuppressions { file, suppressions: file_suppressions });
            }

            let scheme = session.backends.of(file).external_scheme();
            let mut targets = Vec::new();
            for (import, resolved) in parsed.imports.into_iter().zip(resolved) {
                // The node that the import resolved to, if it is in the graph, and that
                // it is imported, unless it is a whole package, imported on its own.
                let target: Option<NodeHandle> = match resolved {
                    ResolvedImport::File(index) => {
                        let target = handles[index as usize].into();
                        builder.add_import(handle, target, import);
                        Some(target)
                    }
                    ResolvedImport::Other(Resolved::Internal(target)) => {
                        let target = builder.add_file(&target)?.into();
                        builder.add_import(handle, target, import);
                        Some(target)
                    }
                    ResolvedImport::Other(Resolved::Package(dir)) => {
                        Some(builder.add_package_import(handle, &dir, import)?)
                    }
                    ResolvedImport::Other(Resolved::External { package, found }) => {
                        missing_packages += usize::from(!found);
                        let target = builder.add_external(&format!("{scheme}:{package}"))?;
                        builder.add_import(handle, target, import);
                        Some(target)
                    }
                    ResolvedImport::Other(Resolved::Builtin(name)) => {
                        let target = builder.add_builtin(&name)?;
                        builder.add_import(handle, target, import);
                        Some(target)
                    }
                    ResolvedImport::Other(Resolved::Unresolved(reason)) => {
                        let file = file.name.clone();
                        unresolved.push(UnresolvedImport { file, import, reason: *reason });
                        None
                    }
                    ResolvedImport::Other(Resolved::Ignored(_)) => None,
                };
                if through_reexports {
                    targets.push(target);
                }
            }
            if through_reexports {
                file_exports.push(FileExports { file: handle, exports: parsed.exports, targets });
            }
        }

        // Sorting is stable, so each file's imports stay in source order.
        unresolved.sort_by(|a, b| a.file.cmp(&b.file));
        (session.unresolved, session.suppressions) = (unresolved, suppressions);
        (session.stats.non_literal_imports, session.stats.missing_packages) =
            (non_literal_imports, missing_packages);
        let (graph, ids) = builder.finish_with_ids();

        // Through re-exports, each re-export's source is the node its import resolved
        // to, if it did.
        if through_reexports {
            let exports: ModuleExports = file_exports
                .into_iter()
                .map(|FileExports { file, exports, targets }| {
                    let exports = exports.map(|import| {
                        let target = (*targets.get(import as usize)?)?;
                        Some(ids.get(target))
                    });
                    (ids.get(file), exports)
                })
                .collect();
            session.graph_through_reexports = Some(graph.through_reexports(&exports));
        }
        session.graph = graph;
        Ok(())
    }
}

/// What a file exports, as the graph through re-exports needs it.
struct FileExports {
    /// The file.
    file: FileHandle,

    /// What it exports, with each re-export's source as the index of its
    /// import.
    exports: Exports<u32>,

    /// The node each of its imports resolved to, in order, if it did.
    targets: Vec<Option<NodeHandle>>,
}
