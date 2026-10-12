//! The view of the graph that a contract checks against.

use zenith_config::{ContractConfig, IgnoreImport};
use zenith_graph::{EdgeSet, Graph, GraphView};
use zenith_workspace::Workspace;

use crate::Selection;

/// The view of the graph that a contract checks against: the graph without
/// the imports that the contract leaves out, and the `ignore_imports` entries
/// that left nothing out.
pub(crate) struct ContractView<'g, 'c> {
    /// The graph without the imports made only by statements of the excluded
    /// kinds, and without the imports that `ignore_imports` matches.
    pub view: GraphView<'g>,

    /// The `ignore_imports` entries that match no import, in the order of the
    /// configuration.
    pub unmatched: Vec<&'c IgnoreImport>,
}

impl<'g, 'c> ContractView<'g, 'c> {
    /// The view of `graph` for the contract `config`, whose `pkg:` selectors
    /// match the `workspace` packages.
    pub(crate) fn new(config: &'c ContractConfig, graph: &'g Graph, workspace: &Workspace) -> Self {
        let mut excluded = EdgeSet::only_of_kinds(graph, config.exclude_kinds);
        let mut unmatched = Vec::new();
        for ignore in &config.ignore_imports {
            // Selectors in `ignore_imports` may match nothing: the entry is reported
            // on its own instead.
            let importers = Selection::resolve(ignore.importer(), graph, workspace).members();
            let imported = Selection::resolve(ignore.imported(), graph, workspace).members();
            let mut matched = false;
            for importer in importers.iter() {
                for target in graph.imports_of(importer) {
                    if imported.contains(*target) {
                        excluded.insert(importer, *target);
                        matched = true;
                    }
                }
            }
            if !matched {
                unmatched.push(ignore);
            }
        }
        Self { view: GraphView::with_excluded(graph, excluded), unmatched }
    }
}
