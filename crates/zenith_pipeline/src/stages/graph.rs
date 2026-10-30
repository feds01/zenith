//! The [`BuildGraph`] stage.

use std::{fmt::Write, ops::Range};

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
        let through_reexports = session.needs_graph_through_reexports();
        let (mut unresolved, mut suppressions, mut file_exports) =
            (Vec::new(), Vec::new(), Vec::new());
        // The node that each import of every file resolved to, if it did, as the graph
        // through re-exports needs them, with each file's in a range.
        let mut targets = Vec::new();
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
        // The graph takes a copy of each import, which shares its names with the
        // session's.
        let files = session.files.iter().zip(&handles);
        let mut external = String::new();
        for (((file, &handle), parsed), resolved) in
            files.zip(&session.parsed).zip(&session.resolved)
        {
            if !parsed.directives.is_empty() {
                builder.set_directives(handle, parsed.directives.clone());
            }
            non_literal_imports += parsed.non_literal_imports.len();
            if !parsed.suppressions.is_empty() {
                // The files are sorted, so the suppressions are too. Each name is the id
                // of the contract it names, as a member's files may name its contracts
                // by their own ids.
                let mut file_suppressions = parsed.suppressions.clone();
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
            let start = targets.len();
            for (import, resolved) in parsed.imports.iter().zip(resolved) {
                // The node that the import resolved to, if it is in the graph, and that
                // it is imported, unless it is a whole package, imported on its own.
                let target: Option<NodeHandle> = match resolved {
                    ResolvedImport::File(index) => {
                        let target = handles[*index as usize].into();
                        builder.add_import(handle, target, import.clone());
                        Some(target)
                    }
                    ResolvedImport::Other(Resolved::Internal(target)) => {
                        let target = builder.add_file(target)?.into();
                        builder.add_import(handle, target, import.clone());
                        Some(target)
                    }
                    ResolvedImport::Other(Resolved::Package(dir)) => {
                        Some(builder.add_package_import(handle, dir, import.clone())?)
                    }
                    ResolvedImport::Other(Resolved::External { package, found }) => {
                        missing_packages += usize::from(!found);
                        external.clear();
                        _ = write!(external, "{scheme}:{package}");
                        let target = builder.add_external(&external)?;
                        builder.add_import(handle, target, import.clone());
                        Some(target)
                    }
                    ResolvedImport::Other(Resolved::Builtin(name)) => {
                        let target = builder.add_builtin(name)?;
                        builder.add_import(handle, target, import.clone());
                        Some(target)
                    }
                    ResolvedImport::Other(Resolved::Unresolved(reason)) => {
                        let file = file.name.clone();
                        let reason = (**reason).clone();
                        let import = import.clone();
                        unresolved.push(UnresolvedImport { file, import, reason });
                        None
                    }
                    ResolvedImport::Other(Resolved::Ignored(_)) => None,
                };
                if through_reexports {
                    targets.push(target);
                }
            }
            if through_reexports {
                let targets = start..targets.len();
                file_exports.push(FileExports { file: handle, exports: &parsed.exports, targets });
            }
        }

        // Sorting is stable, so each file's imports stay in source order.
        unresolved.sort_by(|a, b| a.file.cmp(&b.file));
        let (graph, ids) = builder.finish_with_ids();

        // Through re-exports, each re-export's source is the node its import resolved
        // to, if it did.
        if through_reexports {
            let exports: ModuleExports = file_exports
                .into_iter()
                .map(|FileExports { file, exports, targets: range }| {
                    let file_targets = &targets[range];
                    let exports = exports.map(|import| {
                        let target = (*file_targets.get(import as usize)?)?;
                        Some(ids.get(target))
                    });
                    (ids.get(file), exports)
                })
                .collect();
            session.graph_through_reexports = Some(graph.through_reexports(&exports));
        }
        (session.unresolved, session.suppressions) = (unresolved, suppressions);
        (session.stats.non_literal_imports, session.stats.missing_packages) =
            (non_literal_imports, missing_packages);
        session.graph = graph;
        Ok(())
    }
}

/// What a file exports, as the graph through re-exports needs it.
struct FileExports<'s> {
    /// The file.
    file: FileHandle,

    /// What it exports, with each re-export's source as the index of its
    /// import.
    exports: &'s Exports<u32>,

    /// Where the node that each of its imports resolved to, in order, if it
    /// did, is in the targets of every file.
    targets: Range<usize>,
}
