//! Shortest import chains between sets of nodes, ported from grimp's
//! `pathfinding.rs` and `import_chain_queries.rs`.
//!
//! A chain is a path of imports: `[a, b, c]` means that `a` imports `b` and
//! `b` imports `c`. The searches are methods of [`GraphView`], so they never
//! use the imports that a contract ignores.

use std::iter;

use rustc_hash::FxHashMap;

use crate::{EdgeSet, GraphView, ModuleId, NodeSet};

impl GraphView<'_> {
    /// Find a shortest import chain from a member of `from` to a member of
    /// `to`.
    ///
    /// This is grimp's bidirectional breadth-first search: it expands one
    /// level of imports forwards from `from`, then one level of importers
    /// backwards from `to`, until the two searches meet. Nodes are expanded
    /// in ascending id order, so the chain found is deterministic.
    ///
    /// No chain passes through a member of `excluded_nodes`, or uses an
    /// import in `excluded_imports` (on top of those the view leaves out).
    /// The ends of a chain are never excluded, so `excluded_nodes` may
    /// contain members of `from` and `to`.
    ///
    /// Returns `None` if there is no chain, or if `from` and `to` share a
    /// node: overlapping sets don't describe an import between them, so
    /// contracts skip them.
    pub fn find_shortest_chain(
        &self,
        from: &NodeSet,
        to: &NodeSet,
        excluded_nodes: &NodeSet,
        excluded_imports: &EdgeSet,
    ) -> Option<Vec<ModuleId>> {
        if from.intersects(to) {
            return None;
        }
        ChainSearch::default().run(self, from, to, excluded_nodes, excluded_imports)
    }

    /// Find the shortest import chains from `from` to `to`, as grimp's
    /// `_find_shortest_chains` does.
    ///
    /// After each chain is found, its imports are excluded and the search
    /// runs again, until no chain is left. So no import appears in two
    /// chains, and the chains are in the order they were found: every direct
    /// import first, in ascending order, then the indirect chains, shortest
    /// first.
    ///
    /// Chains never pass through `excluded_nodes`, as in
    /// [`Self::find_shortest_chain`]. Overlapping `from` and `to` have no
    /// chains.
    pub fn find_shortest_chains(
        &self,
        from: &NodeSet,
        to: &NodeSet,
        excluded_nodes: &NodeSet,
    ) -> Vec<Vec<ModuleId>> {
        let (direct, indirect) = ChainSearch::default().find_chains(self, from, to, excluded_nodes);
        direct.into_iter().map(|(a, b)| vec![a, b]).chain(indirect).collect()
    }

    /// Every node that imports a member of `targets`, directly or through
    /// other nodes (grimp's `find_downstream_modules`).
    ///
    /// As in grimp, the members of `targets` are never part of the result,
    /// even if they import each other.
    pub fn find_downstream(&self, targets: &NodeSet) -> NodeSet {
        let mut seen = targets.clone();
        let mut stack: Vec<ModuleId> = targets.iter().collect();
        while let Some(module) = stack.pop() {
            for importer in self.importers_of(module) {
                if seen.insert(importer) {
                    stack.push(importer);
                }
            }
        }
        seen.difference_with(targets);
        seen
    }

    /// Every direct import from a member of `from` to a member of `to`, in
    /// ascending order.
    pub(crate) fn direct_imports<'a>(
        &'a self,
        from: &'a NodeSet,
        to: &'a NodeSet,
    ) -> impl Iterator<Item = (ModuleId, ModuleId)> + 'a {
        from.iter().flat_map(move |importer| {
            self.imports_of(importer)
                .filter(|imported| to.contains(*imported))
                .map(move |imported| (importer, imported))
        })
    }
}

/// The state of a bidirectional search, kept between runs so that repeated
/// searches reuse their allocations.
#[derive(Debug, Default)]
pub(crate) struct ChainSearch {
    /// The nodes reached forwards from `from`, in the order they were
    /// reached. Expanding a node reaches the nodes it imports.
    forwards: Vec<ModuleId>,

    /// The nodes reached backwards from `to`, in the order they were
    /// reached. Expanding a node reaches the nodes that import it.
    backwards: Vec<ModuleId>,

    /// The nodes in `forwards`.
    seen_forwards: NodeSet,

    /// The nodes in `backwards`.
    seen_backwards: NodeSet,

    /// The node each node in `forwards` was first reached from, which
    /// imports it. Members of `from` have none.
    importers: FxHashMap<ModuleId, ModuleId>,

    /// The node each node in `backwards` was first reached from, which it
    /// imports. Members of `to` have none.
    imported: FxHashMap<ModuleId, ModuleId>,
}

impl ChainSearch {
    /// The chains of [`GraphView::find_shortest_chains`], split into the
    /// direct imports and the indirect chains, which have at least one node
    /// between their ends.
    ///
    /// A caller that runs many searches can keep one `ChainSearch` to reuse
    /// its buffers.
    pub(crate) fn find_chains(
        &mut self,
        view: &GraphView,
        from: &NodeSet,
        to: &NodeSet,
        excluded_nodes: &NodeSet,
    ) -> (Vec<(ModuleId, ModuleId)>, Vec<Vec<ModuleId>>) {
        if from.intersects(to) {
            return (Vec::new(), Vec::new());
        }

        // While a direct import is left, the first level of the search finds
        // the lowest one, so repeated searches find every direct import
        // first, in ascending order. Collecting them in one pass gives the
        // same chains without a search for each.
        let direct: Vec<_> = view.direct_imports(from, to).collect();
        let mut excluded_imports: EdgeSet = direct.iter().copied().collect();

        let mut indirect = Vec::new();
        while let Some(chain) = self.run(view, from, to, excluded_nodes, &excluded_imports) {
            excluded_imports.extend(chain.array_windows().map(|&[a, b]| (a, b)));
            indirect.push(chain);
        }
        (direct, indirect)
    }

    /// Find a shortest chain from `from` to `to`, which must not overlap.
    fn run(
        &mut self,
        view: &GraphView,
        from: &NodeSet,
        to: &NodeSet,
        excluded_nodes: &NodeSet,
        excluded_imports: &EdgeSet,
    ) -> Option<Vec<ModuleId>> {
        if from.is_empty() || to.is_empty() {
            return None;
        }
        self.forwards.clear();
        self.forwards.extend(from.iter());
        self.backwards.clear();
        self.backwards.extend(to.iter());
        self.seen_forwards.clone_from(from);
        self.seen_backwards.clone_from(to);
        self.importers.clear();
        self.imported.clear();

        let middle = self.meet(view, excluded_nodes, excluded_imports)?;
        Some(self.chain_through(middle))
    }

    /// Expand the two searches a level at a time until they meet, and return
    /// the node where they do.
    ///
    /// A node is checked against the other search before the exclusions, so
    /// a chain can always end at a member of `from` or `to`. Once either
    /// search runs out of nodes to expand, there is no chain: it would have
    /// reached the other search's start.
    fn meet(
        &mut self,
        view: &GraphView,
        excluded_nodes: &NodeSet,
        excluded_imports: &EdgeSet,
    ) -> Option<ModuleId> {
        let mut expanded_forwards = 0;
        let mut expanded_backwards = 0;
        loop {
            let level_end = self.forwards.len();
            while expanded_forwards < level_end {
                let importer = self.forwards[expanded_forwards];
                expanded_forwards += 1;
                for imported in view.imports_of(importer) {
                    if excluded_imports.contains(importer, imported) {
                        continue;
                    }
                    if self.seen_backwards.contains(imported) {
                        self.importers.insert(imported, importer);
                        return Some(imported);
                    }
                    if !excluded_nodes.contains(imported) && self.seen_forwards.insert(imported) {
                        self.importers.insert(imported, importer);
                        self.forwards.push(imported);
                    }
                }
            }
            if expanded_forwards == self.forwards.len() {
                return None;
            }

            let level_end = self.backwards.len();
            while expanded_backwards < level_end {
                let imported = self.backwards[expanded_backwards];
                expanded_backwards += 1;
                for importer in view.importers_of(imported) {
                    if excluded_imports.contains(importer, imported) {
                        continue;
                    }
                    if self.seen_forwards.contains(importer) {
                        self.imported.insert(importer, imported);
                        return Some(importer);
                    }
                    if !excluded_nodes.contains(importer) && self.seen_backwards.insert(importer) {
                        self.imported.insert(importer, imported);
                        self.backwards.push(importer);
                    }
                }
            }
            if expanded_backwards == self.backwards.len() {
                return None;
            }
        }
    }

    /// The chain through `middle`, where the searches met: the way back to
    /// `from`, then the way on to `to`.
    fn chain_through(&self, middle: ModuleId) -> Vec<ModuleId> {
        let mut chain: Vec<_> =
            iter::successors(Some(middle), |node| self.importers.get(node).copied()).collect();
        chain.reverse();
        chain.extend(iter::successors(self.imported.get(&middle).copied(), |node| {
            self.imported.get(node).copied()
        }));
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Graph,
        testing::{graph_of, id, in_both_orders, names, random_imports, seeded_rng},
    };

    /// The set of the nodes called `names`.
    fn set(graph: &Graph, names: &[&str]) -> NodeSet {
        names.iter().map(|name| id(graph, name)).collect()
    }

    /// The shortest chain between two sets of named nodes, by name.
    fn shortest<'g>(
        graph: &'g Graph,
        from: &[&str],
        to: &[&str],
        excluded: &[&str],
    ) -> Option<Vec<&'g str>> {
        let view = GraphView::new(graph);
        let (from, to, excluded) = (set(graph, from), set(graph, to), set(graph, excluded));
        view.find_shortest_chain(&from, &to, &excluded, &EdgeSet::new())
            .map(|chain| names(graph, chain))
    }

    /// The shortest chains between the members of two nodes, by name.
    fn chains<'g>(view: &GraphView<'g>, from: &str, to: &str) -> Vec<Vec<&'g str>> {
        let graph = view.graph();
        let (from, to) = (graph.members(id(graph, from)), graph.members(id(graph, to)));
        view.find_shortest_chains(&from, &to, &NodeSet::new())
            .into_iter()
            .map(|chain| names(graph, chain))
            .collect()
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
        assert_eq!(shortest(&graph, &["a.ts"], &["d.ts"], &[]).unwrap(), ["a.ts", "x.ts", "d.ts"]);
        assert_eq!(shortest(&graph, &["b.ts"], &["d.ts"], &[]).unwrap(), ["b.ts", "c.ts", "d.ts"]);
        assert_eq!(shortest(&graph, &["c.ts"], &["d.ts"], &[]).unwrap(), ["c.ts", "d.ts"]);
    }

    #[test]
    fn excluded_nodes_and_imports_force_a_longer_chain() {
        let graph = graph_of(&[
            ("a.ts", "b.ts"),
            ("b.ts", "c.ts"),
            ("c.ts", "d.ts"),
            ("a.ts", "x.ts"),
            ("x.ts", "d.ts"),
        ]);
        let long = ["a.ts", "b.ts", "c.ts", "d.ts"];
        assert_eq!(shortest(&graph, &["a.ts"], &["d.ts"], &["x.ts"]).unwrap(), long);
        assert_eq!(shortest(&graph, &["a.ts"], &["d.ts"], &["x.ts", "c.ts"]), None);

        // Excluding either import of the short chain, from either side of
        // the search.
        let view = GraphView::new(&graph);
        let [a, d, x] = ["a.ts", "d.ts", "x.ts"].map(|name| id(&graph, name));
        let none = NodeSet::new();
        for import in [(a, x), (x, d)] {
            let excluded: EdgeSet = [import].into_iter().collect();
            let chain = view.find_shortest_chain(
                &NodeSet::single(a),
                &NodeSet::single(d),
                &none,
                &excluded,
            );
            assert_eq!(names(&graph, chain.unwrap()), long);
        }
    }

    #[test]
    fn finds_a_direct_import_when_there_is_one() {
        let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "c.ts"), ("a.ts", "c.ts")]);
        assert_eq!(shortest(&graph, &["a.ts"], &["c.ts"], &[]).unwrap(), ["a.ts", "c.ts"]);
    }

    #[test]
    fn finds_no_chain_against_the_direction_of_imports() {
        let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "c.ts"), ("d.ts", "c.ts")]);
        assert_eq!(shortest(&graph, &["c.ts"], &["a.ts"], &[]), None);
        assert_eq!(shortest(&graph, &["a.ts"], &["d.ts"], &[]), None);
    }

    #[test]
    fn copes_with_cycles() {
        let graph = graph_of(&[
            ("a.ts", "b.ts"),
            ("b.ts", "a.ts"),
            ("b.ts", "c.ts"),
            ("c.ts", "b.ts"),
            ("c.ts", "d.ts"),
        ]);
        assert_eq!(
            shortest(&graph, &["a.ts"], &["d.ts"], &[]).unwrap(),
            ["a.ts", "b.ts", "c.ts", "d.ts"]
        );
        assert_eq!(shortest(&graph, &["d.ts"], &["a.ts"], &[]), None);
    }

    #[test]
    fn overlapping_or_empty_sets_have_no_chain() {
        let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "c.ts")]);
        let view = GraphView::new(&graph);
        let [a, b, c] = ["a.ts", "b.ts", "c.ts"].map(|name| id(&graph, name));
        let none = NodeSet::new();

        let from: NodeSet = [a, b].into_iter().collect();
        let to: NodeSet = [b, c].into_iter().collect();
        assert_eq!(view.find_shortest_chain(&from, &to, &none, &EdgeSet::new()), None);
        assert!(view.find_shortest_chains(&from, &to, &none).is_empty());

        let a = NodeSet::single(a);
        assert_eq!(view.find_shortest_chain(&none, &a, &none, &EdgeSet::new()), None);
        assert_eq!(view.find_shortest_chain(&a, &none, &none, &EdgeSet::new()), None);
        assert!(view.find_shortest_chains(&a, &none, &none).is_empty());
    }

    #[test]
    fn the_ends_of_a_chain_are_never_excluded() {
        let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "c.ts"), ("a.ts", "x.ts")]);
        assert_eq!(
            shortest(&graph, &["a.ts"], &["c.ts"], &["a.ts", "c.ts"]).unwrap(),
            ["a.ts", "b.ts", "c.ts"]
        );
        assert_eq!(shortest(&graph, &["a.ts"], &["c.ts"], &["a.ts", "b.ts", "c.ts"]), None);
        assert_eq!(
            shortest(&graph, &["a.ts"], &["b.ts"], &["a.ts", "b.ts"]).unwrap(),
            ["a.ts", "b.ts"]
        );
    }

    #[test]
    fn chains_of_equal_length_go_through_the_lowest_ids() {
        // Two chains of the same length, whichever order they are added in.
        let imports = [("a.ts", "c.ts"), ("c.ts", "d.ts"), ("a.ts", "b.ts"), ("b.ts", "d.ts")];
        let chain = in_both_orders(&imports, |graph| {
            shortest(graph, &["a.ts"], &["d.ts"], &[]).map(|chain| chain.join(" -> "))
        });
        assert_eq!(chain.unwrap(), "a.ts -> b.ts -> d.ts");

        // Several starts and ends: the lowest of each.
        let graph = graph_of(&[("a.ts", "y.ts"), ("b.ts", "x.ts"), ("a.ts", "x.ts")]);
        assert_eq!(
            shortest(&graph, &["a.ts", "b.ts"], &["x.ts", "y.ts"], &[]).unwrap(),
            ["a.ts", "x.ts"]
        );
    }

    #[test]
    fn finds_every_chain_without_reusing_imports() {
        let graph = graph_of(&[
            ("a.ts", "b.ts"),
            ("b.ts", "d.ts"),
            ("a.ts", "c.ts"),
            ("c.ts", "d.ts"),
            ("a.ts", "d.ts"),
            ("c.ts", "e.ts"),
            ("e.ts", "d.ts"),
        ]);
        let view = GraphView::new(&graph);
        // `a -> c -> e -> d` reuses `a -> c`, so it is not a chain of its own.
        assert_eq!(
            chains(&view, "a.ts", "d.ts"),
            [vec!["a.ts", "d.ts"], vec!["a.ts", "b.ts", "d.ts"], vec!["a.ts", "c.ts", "d.ts"]]
        );
    }

    #[test]
    fn chains_between_members_of_the_same_end_are_not_included() {
        let graph = graph_of(&[
            ("importer/one.ts", "importer/two.ts"),
            ("importer/two.ts", "importer/three.ts"),
            ("importer/three.ts", "imported/four.ts"),
            ("imported/four.ts", "imported/five.ts"),
            ("imported/five.ts", "imported/six.ts"),
        ]);
        let view = GraphView::new(&graph);
        assert_eq!(
            chains(&view, "importer", "imported"),
            [vec!["importer/three.ts", "imported/four.ts"]]
        );
    }

    #[test]
    fn chains_through_the_importer_do_not_stop_longer_chains() {
        let graph = graph_of(&[
            ("green/foo.ts", "blue/foo.ts"),
            ("green/baz.ts", "green/foo.ts"),
            ("green/baz.ts", "yellow/three.ts"),
            ("yellow/three.ts", "yellow/two.ts"),
            ("yellow/two.ts", "yellow/one.ts"),
            ("yellow/one.ts", "blue/bar.ts"),
        ]);
        let view = GraphView::new(&graph);
        assert_eq!(
            chains(&view, "green", "blue"),
            [
                vec!["green/foo.ts", "blue/foo.ts"],
                vec![
                    "green/baz.ts",
                    "yellow/three.ts",
                    "yellow/two.ts",
                    "yellow/one.ts",
                    "blue/bar.ts"
                ],
            ]
        );
    }

    #[test]
    fn chains_that_reenter_an_end_do_not_stop_longer_chains() {
        // Through `brown`, back into the importer.
        let graph = graph_of(&[
            ("green/baz.ts", "brown.ts"),
            ("brown.ts", "green/foo.ts"),
            ("green/foo.ts", "blue/foo.ts"),
            ("green/baz.ts", "yellow/three.ts"),
            ("yellow/three.ts", "yellow/two.ts"),
            ("yellow/two.ts", "yellow/one.ts"),
            ("yellow/one.ts", "blue/foo.ts"),
        ]);
        let view = GraphView::new(&graph);
        assert_eq!(
            chains(&view, "green", "blue"),
            [
                vec!["green/foo.ts", "blue/foo.ts"],
                vec![
                    "green/baz.ts",
                    "yellow/three.ts",
                    "yellow/two.ts",
                    "yellow/one.ts",
                    "blue/foo.ts"
                ],
            ]
        );

        // Through `brown`, back into the imported.
        let graph = graph_of(&[
            ("green/foo.ts", "blue/foo.ts"),
            ("blue/foo.ts", "brown.ts"),
            ("brown.ts", "blue/bar.ts"),
            ("green/foo.ts", "yellow/four.ts"),
            ("yellow/four.ts", "yellow/three.ts"),
            ("yellow/three.ts", "yellow/two.ts"),
            ("yellow/two.ts", "yellow/one.ts"),
            ("yellow/one.ts", "blue/bar.ts"),
        ]);
        let view = GraphView::new(&graph);
        assert_eq!(
            chains(&view, "green", "blue"),
            [
                vec!["green/foo.ts", "blue/foo.ts"],
                vec![
                    "green/foo.ts",
                    "yellow/four.ts",
                    "yellow/three.ts",
                    "yellow/two.ts",
                    "yellow/one.ts",
                    "blue/bar.ts"
                ],
            ]
        );
    }

    #[test]
    fn chains_avoid_excluded_nodes() {
        let graph = graph_of(&[
            ("low/a.ts", "mid/b.ts"),
            ("mid/b.ts", "high/c.ts"),
            ("low/a.ts", "utils/d.ts"),
            ("utils/d.ts", "high/c.ts"),
            ("low/e.ts", "high/c.ts"),
        ]);
        let view = GraphView::new(&graph);
        let [low, mid, high] = ["low", "mid", "high"].map(|name| graph.members(id(&graph, name)));
        let chains = view.find_shortest_chains(&low, &high, &mid);
        assert_eq!(
            chains.into_iter().map(|chain| names(&graph, chain)).collect::<Vec<_>>(),
            [vec!["low/e.ts", "high/c.ts"], vec!["low/a.ts", "utils/d.ts", "high/c.ts"]]
        );
    }

    #[test]
    fn chains_respect_the_imports_a_view_leaves_out() {
        let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "c.ts"), ("a.ts", "c.ts")]);
        let [a, b, c] = ["a.ts", "b.ts", "c.ts"].map(|name| id(&graph, name));

        let view = GraphView::with_excluded(&graph, [(a, c)].into_iter().collect());
        assert_eq!(chains(&view, "a.ts", "c.ts"), [vec!["a.ts", "b.ts", "c.ts"]]);

        let view = GraphView::with_excluded(&graph, [(a, c), (b, c)].into_iter().collect());
        assert!(chains(&view, "a.ts", "c.ts").is_empty());
        let chain = view.find_shortest_chain(
            &NodeSet::single(a),
            &NodeSet::single(c),
            &NodeSet::new(),
            &EdgeSet::new(),
        );
        assert_eq!(chain, None);
    }

    #[test]
    fn chains_between_directories_and_to_externals() {
        let graph = graph_of(&[
            ("low/one.ts", "mid/two.ts"),
            ("mid/two.ts", "high/three.ts"),
            ("low/four.ts", "npm:react"),
        ]);
        let view = GraphView::new(&graph);
        assert_eq!(
            chains(&view, "low", "high"),
            [vec!["low/one.ts", "mid/two.ts", "high/three.ts"]]
        );
        assert_eq!(chains(&view, "low", "npm:react"), [vec!["low/four.ts", "npm:react"]]);
        assert!(chains(&view, "high", "low").is_empty());
    }

    /// The chains of grimp's `_find_shortest_chains`, which runs a search for
    /// every chain, the direct imports included.
    fn repeated_searches(
        view: &GraphView,
        from: &NodeSet,
        to: &NodeSet,
        excluded_nodes: &NodeSet,
    ) -> Vec<Vec<ModuleId>> {
        let mut chains = Vec::new();
        let mut excluded = EdgeSet::new();
        while let Some(chain) = view.find_shortest_chain(from, to, excluded_nodes, &excluded) {
            excluded.extend(chain.array_windows().map(|&[a, b]| (a, b)));
            chains.push(chain);
        }
        chains
    }

    /// The length of a shortest chain from `from` to `to` by a plain
    /// breadth-first search, avoiding `excluded` between the ends.
    fn reference_length(
        view: &GraphView,
        from: &NodeSet,
        to: &NodeSet,
        excluded: &NodeSet,
    ) -> Option<usize> {
        let mut seen = from.clone();
        let mut level: Vec<ModuleId> = from.iter().collect();
        for length in 1.. {
            let mut next = Vec::new();
            for importer in level {
                for imported in view.imports_of(importer) {
                    if to.contains(imported) {
                        return Some(length);
                    }
                    if !excluded.contains(imported) && seen.insert(imported) {
                        next.push(imported);
                    }
                }
            }
            if next.is_empty() {
                return None;
            }
            level = next;
        }
        None
    }

    #[test]
    fn chains_are_valid_and_shortest_on_generated_graphs() {
        let mut random = seeded_rng();
        let files: Vec<String> = (0..12).map(|index| format!("src/{index:02}.ts")).collect();

        for _ in 0..300 {
            let count = random.usize(..30);
            let imports = random_imports(&mut random, &files, count);
            if imports.is_empty() {
                continue;
            }
            let graph = graph_of(&imports);
            let view = GraphView::new(&graph);
            let (from_count, to_count, excluded_count) =
                (1 + random.usize(..3), 1 + random.usize(..3), random.usize(..4));
            let mut pick = |count: usize| -> NodeSet {
                (0..count).filter_map(|_| graph.id(&files[random.usize(..files.len())])).collect()
            };
            let from = pick(from_count);
            let mut to = pick(to_count);
            to.difference_with(&from);
            let excluded = pick(excluded_count);

            let chain = view.find_shortest_chain(&from, &to, &excluded, &EdgeSet::new());
            let expected = reference_length(&view, &from, &to, &excluded);
            assert_eq!(chain.as_ref().map(|chain| chain.len() - 1), expected, "{imports:?}");

            // Collecting the direct imports up front must give the same chains,
            // in order, as a search for every chain.
            let chains = view.find_shortest_chains(&from, &to, &excluded);
            assert_eq!(chains, repeated_searches(&view, &from, &to, &excluded), "{imports:?}");
            assert_eq!(chains.is_empty(), expected.is_none());
            let mut used = EdgeSet::new();
            for chain in &chains {
                let (first, last) = (chain[0], chain[chain.len() - 1]);
                assert!(from.contains(first) && to.contains(last), "{chain:?}");
                for middle in &chain[1..chain.len() - 1] {
                    assert!(!excluded.contains(*middle) && !from.contains(*middle));
                    assert!(!to.contains(*middle));
                }
                for &[importer, imported] in chain.array_windows() {
                    assert!(graph.has_import(importer, imported));
                    assert!(used.insert(importer, imported), "an import is in two chains");
                }
            }
        }
    }

    #[test]
    fn chains_do_not_depend_on_the_order_the_graph_was_built_in() {
        let imports = [
            ("web/src/pages/a.ts", "web/src/lib/x.ts"),
            ("web/src/pages/a.ts", "web/src/lib/y.ts"),
            ("web/src/pages/b.ts", "web/src/lib/y.ts"),
            ("web/src/lib/x.ts", "npm:react"),
            ("web/src/lib/y.ts", "npm:react"),
            ("web/src/pages/b.ts", "npm:react"),
        ];
        let chains = in_both_orders(&imports, |graph| {
            let chains = chains(&GraphView::new(graph), "web/src/pages", "npm:react");
            chains.into_iter().map(|chain| chain.join(" -> ")).collect::<Vec<_>>()
        });
        assert_eq!(
            chains,
            [
                "web/src/pages/b.ts -> npm:react",
                "web/src/pages/a.ts -> web/src/lib/x.ts -> npm:react",
                "web/src/pages/a.ts -> web/src/lib/y.ts -> npm:react",
            ]
        );
    }

    #[test]
    fn downstream_nodes_import_the_targets_directly_or_indirectly() {
        let graph = graph_of(&[
            ("a.ts", "b.ts"),
            ("b.ts", "t1.ts"),
            ("c.ts", "t1.ts"),
            ("t2.ts", "t1.ts"),
            ("t1.ts", "d.ts"),
            ("e.ts", "d.ts"),
            ("f.ts", "a.ts"),
            ("a.ts", "f.ts"),
        ]);
        let view = GraphView::new(&graph);
        let targets = set(&graph, &["t1.ts", "t2.ts"]);
        assert_eq!(
            names(&graph, view.find_downstream(&targets).iter()),
            ["a.ts", "b.ts", "c.ts", "f.ts"]
        );
        assert!(view.find_downstream(&set(&graph, &["f.ts"])).contains(id(&graph, "a.ts")));
        assert!(view.find_downstream(&set(&graph, &["c.ts"])).is_empty());
        assert!(view.find_downstream(&NodeSet::new()).is_empty());

        let [b, t1] = ["b.ts", "t1.ts"].map(|name| id(&graph, name));
        let view = GraphView::with_excluded(&graph, [(b, t1)].into_iter().collect());
        assert_eq!(names(&graph, view.find_downstream(&targets).iter()), ["c.ts"]);
    }
}
