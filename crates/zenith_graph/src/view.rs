//! A filtered view of a [Graph] with some imports excluded.

use rustc_hash::FxHashSet;

use crate::{Graph, ModuleId};

/// A set of `(importer, imported)` pairs.
pub type EdgeSet = FxHashSet<(ModuleId, ModuleId)>;

/// A view of a [Graph] in which a set of imports does not exist.
///
/// Each contract checks against its own view, built from its ignored imports,
/// baseline entries, suppressions and excluded import kinds. The underlying
/// graph is shared and never mutated.
#[derive(Debug, Clone)]
pub struct GraphView<'g> {
    graph: &'g Graph,
    excluded: EdgeSet,
}

impl<'g> GraphView<'g> {
    /// A view of the whole graph.
    pub fn new(graph: &'g Graph) -> Self {
        Self { graph, excluded: EdgeSet::default() }
    }

    /// A view of the graph without the `excluded` imports.
    pub fn with_excluded(graph: &'g Graph, excluded: EdgeSet) -> Self {
        Self { graph, excluded }
    }

    /// The underlying graph.
    pub fn graph(&self) -> &'g Graph {
        self.graph
    }

    /// The imports excluded from this view.
    pub fn excluded(&self) -> &EdgeSet {
        &self.excluded
    }

    /// Whether the import `importer -> imported` exists in this view.
    pub fn has_import(&self, importer: ModuleId, imported: ModuleId) -> bool {
        self.graph.has_import(importer, imported) && !self.excluded.contains(&(importer, imported))
    }

    /// The modules directly imported by `id` in this view.
    pub fn imports_of(&self, id: ModuleId) -> impl Iterator<Item = ModuleId> + '_ {
        self.graph
            .imports_of(id)
            .iter()
            .copied()
            .filter(move |imported| !self.excluded.contains(&(id, *imported)))
    }

    /// The modules that directly import `id` in this view.
    pub fn importers_of(&self, id: ModuleId) -> impl Iterator<Item = ModuleId> + '_ {
        self.graph
            .importers_of(id)
            .iter()
            .copied()
            .filter(move |importer| !self.excluded.contains(&(*importer, id)))
    }
}
