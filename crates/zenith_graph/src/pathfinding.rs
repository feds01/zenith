//! Shortest import chains, ported from grimp's `pathfinding.rs` and
//! `import_chain_queries.rs`.

use crate::{EdgeSet, GraphView, ModuleId, NodeSet};

/// A safety limit on the number of chains [find_shortest_chains] collects
/// between two sets of modules.
const MAX_CHAINS: usize = 1000;

/// Find the shortest import chain from any module in `from` to any module in
/// `to`, using a bidirectional breadth-first search.
///
/// Chains may not pass through `excluded_nodes`, nor use the imports in
/// `excluded_imports` (on top of those the view already excludes). The
/// returned chain starts with a member of `from` and ends with a member of
/// `to`. `from` and `to` must be disjoint; if they are not, no chain is
/// returned.
pub fn find_shortest_chain(
    view: &GraphView,
    from: &NodeSet,
    to: &NodeSet,
    excluded_nodes: &NodeSet,
    excluded_imports: &EdgeSet,
) -> Option<Vec<ModuleId>> {
    if from.intersects(to) {
        return None;
    }

    let is_excluded = |importer: ModuleId, imported: ModuleId| {
        excluded_nodes.contains(importer)
            || excluded_nodes.contains(imported)
            || excluded_imports.contains(&(importer, imported))
    };

    // The queues hold modules in the order they were reached, and the parent
    // maps record which module each was reached from, as in grimp.
    let mut predecessors: Vec<ModuleId> = from.iter().collect();
    let mut successors: Vec<ModuleId> = to.iter().collect();
    let mut seen_forwards: NodeSet = from.clone();
    let mut seen_backwards: NodeSet = to.clone();
    let mut parent_forwards = rustc_hash::FxHashMap::default();
    let mut parent_backwards = rustc_hash::FxHashMap::default();

    let mut i_forwards = 0;
    let mut i_backwards = 0;

    let middle = 'search: loop {
        let frontier_end = predecessors.len();
        while i_forwards < frontier_end {
            let module = predecessors[i_forwards];
            for next in view.imports_of(module) {
                if is_excluded(module, next) {
                    continue;
                }
                if seen_forwards.insert(next) {
                    predecessors.push(next);
                    parent_forwards.insert(next, module);
                }
                if seen_backwards.contains(next) {
                    break 'search Some(next);
                }
            }
            i_forwards += 1;
        }

        let frontier_end = successors.len();
        while i_backwards < frontier_end {
            let module = successors[i_backwards];
            for next in view.importers_of(module) {
                if is_excluded(next, module) {
                    continue;
                }
                if seen_backwards.insert(next) {
                    successors.push(next);
                    parent_backwards.insert(next, module);
                }
                if seen_forwards.contains(next) {
                    break 'search Some(next);
                }
            }
            i_backwards += 1;
        }

        if i_forwards == predecessors.len() && i_backwards == successors.len() {
            break 'search None;
        }
    };

    let middle = middle?;
    let mut path = vec![middle];
    let mut node = middle;
    while let Some(parent) = parent_forwards.get(&node) {
        path.push(*parent);
        node = *parent;
    }
    path.reverse();
    let mut node = middle;
    while let Some(child) = parent_backwards.get(&node) {
        path.push(*child);
        node = *child;
    }
    Some(path)
}

/// Every module that imports a member of `targets`, directly or indirectly
/// (grimp's `find_downstream_modules`). The targets themselves are not
/// included unless they import each other.
pub fn find_downstream(view: &GraphView, targets: &NodeSet) -> NodeSet {
    let mut seen = NodeSet::new();
    let mut queue: Vec<ModuleId> = targets.iter().collect();
    while let Some(module) = queue.pop() {
        for importer in view.importers_of(module) {
            if seen.insert(importer) {
                queue.push(importer);
            }
        }
    }
    seen
}

/// Find the shortest chains from `from` to `to`.
///
/// After each chain is found, its imports are excluded and the search runs
/// again, so every import appears in at most one chain. This is grimp's
/// `_find_shortest_chains`.
pub fn find_shortest_chains(
    view: &GraphView,
    from: &NodeSet,
    to: &NodeSet,
    excluded_nodes: &NodeSet,
) -> Vec<Vec<ModuleId>> {
    let mut chains = Vec::new();
    let mut excluded_imports = EdgeSet::default();

    while chains.len() < MAX_CHAINS {
        let Some(chain) = find_shortest_chain(view, from, to, excluded_nodes, &excluded_imports)
        else {
            break;
        };
        for pair in chain.windows(2) {
            excluded_imports.insert((pair[0], pair[1]));
        }
        chains.push(chain);
    }

    chains
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{graph_of, id};

    fn names(graph: &crate::Graph, chain: &[ModuleId]) -> Vec<String> {
        chain.iter().map(|id| graph.name(*id).to_string()).collect()
    }

    #[test]
    fn finds_the_shortest_chain() {
        let graph = graph_of(&[
            ("a.ts", "b.ts"),
            ("b.ts", "c.ts"),
            ("c.ts", "d.ts"),
            ("a.ts", "x.ts"),
            ("x.ts", "d.ts"),
        ]);
        let view = GraphView::new(&graph);
        let from = NodeSet::single(id(&graph, "a.ts"));
        let to = NodeSet::single(id(&graph, "d.ts"));
        let chain = find_shortest_chain(&view, &from, &to, &NodeSet::new(), &EdgeSet::default());
        assert_eq!(names(&graph, &chain.unwrap()), vec!["a.ts", "x.ts", "d.ts"]);

        // Excluding the short route finds the long one.
        let excluded = NodeSet::single(id(&graph, "x.ts"));
        let chain = find_shortest_chain(&view, &from, &to, &excluded, &EdgeSet::default());
        assert_eq!(names(&graph, &chain.unwrap()), vec!["a.ts", "b.ts", "c.ts", "d.ts"]);
    }

    #[test]
    fn finds_all_shortest_chains_without_reusing_imports() {
        let graph = graph_of(&[
            ("a.ts", "b.ts"),
            ("b.ts", "d.ts"),
            ("a.ts", "c.ts"),
            ("c.ts", "d.ts"),
            ("a.ts", "d.ts"),
        ]);
        let view = GraphView::new(&graph);
        let from = NodeSet::single(id(&graph, "a.ts"));
        let to = NodeSet::single(id(&graph, "d.ts"));
        let chains = find_shortest_chains(&view, &from, &to, &NodeSet::new());
        let chains: Vec<_> = chains.iter().map(|c| names(&graph, c)).collect();
        assert_eq!(
            chains,
            vec![vec!["a.ts", "d.ts"], vec!["a.ts", "b.ts", "d.ts"], vec!["a.ts", "c.ts", "d.ts"],]
        );
    }

    #[test]
    fn respects_view_exclusions() {
        let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "c.ts")]);
        let mut excluded = EdgeSet::default();
        excluded.insert((id(&graph, "a.ts"), id(&graph, "b.ts")));
        let view = GraphView::with_excluded(&graph, excluded);
        let from = NodeSet::single(id(&graph, "a.ts"));
        let to = NodeSet::single(id(&graph, "c.ts"));
        assert!(find_shortest_chains(&view, &from, &to, &NodeSet::new()).is_empty());
    }

    #[test]
    fn package_to_package_chains() {
        let graph = graph_of(&[
            ("low/one.ts", "mid/two.ts"),
            ("mid/two.ts", "high/three.ts"),
            ("low/four.ts", "npm:react"),
        ]);
        let view = GraphView::new(&graph);
        let low = graph.members(id(&graph, "low"));
        let high = graph.members(id(&graph, "high"));
        let chains = find_shortest_chains(&view, &low, &high, &NodeSet::new());
        assert_eq!(chains.len(), 1);
        assert_eq!(names(&graph, &chains[0]), vec!["low/one.ts", "mid/two.ts", "high/three.ts"]);

        let react = NodeSet::single(id(&graph, "npm:react"));
        let chains = find_shortest_chains(&view, &low, &react, &NodeSet::new());
        assert_eq!(names(&graph, &chains[0]), vec!["low/four.ts", "npm:react"]);
    }
}
