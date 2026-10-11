//! A contract's view of a [`Graph`]: the graph with some imports excluded.

use std::fmt;

use rustc_hash::FxHashSet;

use crate::{Graph, ModuleId};

/// A set of imports, each an `(importer, imported)` pair of nodes.
///
/// The set answers membership only. It cannot be iterated, so its hash order
/// can never leak into output.
#[derive(Clone, Default)]
pub struct EdgeSet {
    /// The `(importer, imported)` pairs.
    edges: FxHashSet<(ModuleId, ModuleId)>,
}

impl EdgeSet {
    /// Create an empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the import from `importer` to `imported`. Returns whether it was
    /// newly added.
    pub fn insert(&mut self, importer: ModuleId, imported: ModuleId) -> bool {
        self.edges.insert((importer, imported))
    }

    /// Whether the set contains the import from `importer` to `imported`.
    pub fn contains(&self, importer: ModuleId, imported: ModuleId) -> bool {
        self.edges.contains(&(importer, imported))
    }
}

impl fmt::Debug for EdgeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut edges: Vec<_> = self.edges.iter().map(|(a, b)| (a.index(), b.index())).collect();
        edges.sort_unstable();
        f.debug_set().entries(edges).finish()
    }
}

impl FromIterator<(ModuleId, ModuleId)> for EdgeSet {
    fn from_iter<T: IntoIterator<Item = (ModuleId, ModuleId)>>(iter: T) -> Self {
        Self { edges: iter.into_iter().collect() }
    }
}

impl Extend<(ModuleId, ModuleId)> for EdgeSet {
    fn extend<T: IntoIterator<Item = (ModuleId, ModuleId)>>(&mut self, iter: T) {
        self.edges.extend(iter);
    }
}

/// A view of a [`Graph`] in which some imports do not exist.
///
/// Each contract checks against its own view, which leaves out the
/// `(importer, imported)` pairs the contract ignores: ignored imports,
/// baseline entries, and pairs none of whose details survive the contract's
/// filters (excluded kinds and suppressions). A pair with one surviving
/// detail stays. The graph itself is shared by every view, and is never
/// mutated or copied.
#[derive(Debug, Clone)]
pub struct GraphView<'g> {
    /// The underlying graph.
    graph: &'g Graph,

    /// The imports that do not exist in this view.
    excluded: EdgeSet,
}

impl<'g> GraphView<'g> {
    /// A view of the whole graph.
    pub fn new(graph: &'g Graph) -> Self {
        Self::with_excluded(graph, EdgeSet::new())
    }

    /// A view of the graph without the `excluded` imports.
    pub fn with_excluded(graph: &'g Graph, excluded: EdgeSet) -> Self {
        Self { graph, excluded }
    }

    /// The underlying graph, for the queries that don't depend on imports,
    /// such as names and the module tree.
    pub fn graph(&self) -> &'g Graph {
        self.graph
    }

    /// The nodes that `id` imports directly in this view, in ascending id
    /// order.
    pub fn imports_of(&self, id: ModuleId) -> impl Iterator<Item = ModuleId> + '_ {
        self.graph
            .imports_of(id)
            .iter()
            .copied()
            .filter(move |imported| !self.excluded.contains(id, *imported))
    }

    /// The nodes that import `id` directly in this view, in ascending id
    /// order.
    pub fn importers_of(&self, id: ModuleId) -> impl Iterator<Item = ModuleId> + '_ {
        self.graph
            .importers_of(id)
            .iter()
            .copied()
            .filter(move |importer| !self.excluded.contains(*importer, id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{graph_of, id};

    #[test]
    fn an_unfiltered_view_has_every_import() {
        let graph = graph_of(&[("a.ts", "b.ts"), ("a.ts", "c.ts"), ("b.ts", "c.ts")]);
        let view = GraphView::new(&graph);
        assert!(std::ptr::eq(view.graph(), &graph));
        for node in graph.ids() {
            assert_eq!(view.imports_of(node).collect::<Vec<_>>(), graph.imports_of(node));
            assert_eq!(view.importers_of(node).collect::<Vec<_>>(), graph.importers_of(node));
        }
    }

    #[test]
    fn a_view_hides_excluded_imports_in_both_directions() {
        let graph = graph_of(&[("a.ts", "b.ts"), ("a.ts", "c.ts"), ("b.ts", "c.ts")]);
        let [a, b, c] = ["a.ts", "b.ts", "c.ts"].map(|name| id(&graph, name));

        // `a -> c` is hidden. `c -> a` is not an import, so excluding it
        // changes nothing.
        let view = GraphView::with_excluded(&graph, [(a, c), (c, a)].into_iter().collect());
        assert_eq!(view.imports_of(a).collect::<Vec<_>>(), [b]);
        assert_eq!(view.imports_of(b).collect::<Vec<_>>(), [c]);
        assert_eq!(view.importers_of(c).collect::<Vec<_>>(), [b]);
        assert_eq!(view.importers_of(b).collect::<Vec<_>>(), [a]);
        assert_eq!(view.importers_of(a).count(), 0);
        assert_eq!(view.imports_of(c).count(), 0);

        // The graph itself is untouched.
        assert_eq!(graph.imports_of(a), [b, c]);
        assert_eq!(graph.importers_of(c), [a, b]);
    }

    #[test]
    fn edge_sets_hold_directed_pairs() {
        let [a, b, c] = [1, 2, 3].map(ModuleId::new);
        let mut edges = EdgeSet::new();
        assert!(edges.insert(a, b));
        assert!(!edges.insert(a, b));
        assert!(edges.contains(a, b));
        assert!(!edges.contains(b, a));

        edges.extend([(c, a), (b, c)]);
        assert!(edges.contains(c, a) && edges.contains(b, c));
        assert_eq!(format!("{edges:?}"), "{(1, 2), (2, 3), (3, 1)}");
    }
}
