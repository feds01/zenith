//! Dependency cycles between units, and the dependencies to remove to break
//! them, ported from grimp's `cycle_breakers.rs`.
//!
//! A [`UnitGraph`] aggregates the direct imports between the members of a
//! list of units into weighted dependencies between the units, which are
//! referred to by their [`UnitId`], the index in that list.

use std::{cmp::Reverse, collections::VecDeque, iter, mem};

use index_vec::{IndexSlice, IndexVec, index_vec};
use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::{GraphView, NodeSet};

index_vec::define_index_type! {
    /// A unit of a [`UnitGraph`]: its index in the list of units.
    pub struct UnitId = u32;
    MAX_INDEX = u32::MAX as usize;
}

/// A dependency of one unit on another.
///
/// Dependencies order by importer, then imported unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitDependency {
    /// The importing unit.
    pub importer: UnitId,

    /// The imported unit.
    pub imported: UnitId,

    /// The number of imports that make the dependency: the `(importer,
    /// imported)` pairs of members, however many statements make each pair.
    pub weight: u32,
}

/// The dependencies between a list of disjoint units, aggregated from the
/// direct imports between their members.
#[derive(Debug, Clone, Default)]
pub struct UnitGraph {
    /// The number of units.
    unit_count: usize,

    /// Every dependency, sorted by importer then imported unit.
    dependencies: Vec<UnitDependency>,

    /// Every dependency, sorted by imported then importing unit.
    by_imported: Vec<UnitDependency>,
}

impl UnitGraph {
    /// Aggregate the imports between `units`, which must not overlap.
    ///
    /// Imports within a unit, and imports of nodes outside every unit, are
    /// not dependencies. The order of `units` breaks ties in
    /// [`Self::nominate_cycle_breakers`].
    pub fn build(view: &GraphView, units: &[NodeSet]) -> Self {
        let units: &IndexSlice<UnitId, [NodeSet]> = IndexSlice::new(units);

        // Only the members matter, so look them up in a map of their own
        // rather than a table of every node in the graph.
        let member_count = units.iter().map(NodeSet::len).sum();
        let mut unit_of = FxHashMap::with_capacity_and_hasher(member_count, FxBuildHasher);
        for (index, unit) in units.iter_enumerated() {
            for member in unit.iter() {
                let previous = unit_of.insert(member, index);
                debug_assert!(previous.is_none(), "units must not overlap");
            }
        }

        // Count the imports of one unit at a time into `weights`, indexed by
        // imported unit, so the dependencies come out in order.
        let mut weights: IndexVec<UnitId, u32> = index_vec![0; units.len()];
        let mut targets = Vec::new();
        let mut dependencies = Vec::new();
        for (importer, unit) in units.iter_enumerated() {
            let imported = unit.iter().flat_map(|member| view.imports_of(member));
            for target in imported.filter_map(|node| unit_of.get(&node).copied()) {
                if target != importer {
                    if weights[target] == 0 {
                        targets.push(target);
                    }
                    weights[target] += 1;
                }
            }
            targets.sort_unstable();
            dependencies.extend(targets.drain(..).map(|imported| UnitDependency {
                importer,
                imported,
                weight: mem::take(&mut weights[imported]),
            }));
        }
        Self::new(units.len(), dependencies)
    }

    /// The graph of `unit_count` units with `dependencies`, sorted by
    /// importer then imported unit.
    fn new(unit_count: usize, dependencies: Vec<UnitDependency>) -> Self {
        let mut by_imported = dependencies.clone();
        by_imported.sort_unstable_by_key(|dependency| (dependency.imported, dependency.importer));
        Self { unit_count, dependencies, by_imported }
    }

    /// Every dependency, sorted by importer then imported unit.
    pub fn dependencies(&self) -> &[UnitDependency] {
        &self.dependencies
    }

    /// Nominate the dependencies to remove so that the units become acyclic,
    /// sorted by importer then imported unit. Empty when they already are.
    ///
    /// This is grimp's weighted variant of the Eades–Lin–Smyth heuristic for
    /// the minimum feedback arc set. The units are put in order: first the
    /// sources, which nothing left imports; last the sinks, which import
    /// nothing left; and in between the rest, picking the unit with the
    /// highest weighted out-degree minus in-degree each time. Every
    /// dependency that points backwards in that order, between two units of
    /// the same strongly connected component, is a breaker.
    ///
    /// Each cycle lies within one component, and has a dependency that
    /// points backwards, so every cycle gets a breaker. A dependency between
    /// components is on no cycle, although it may point backwards when the
    /// order interleaves the components, so unlike grimp, zenith never
    /// nominates one.
    ///
    /// Ties go to the unit that comes first, so pass the units in
    /// alphabetical order of their labels to break ties alphabetically, as
    /// grimp does.
    pub fn nominate_cycle_breakers(&self) -> Vec<UnitDependency> {
        let mut peeling = Peeling::new(self);

        // Sources go first. Removing one can make the units it imports
        // sources.
        let mut front = Vec::with_capacity(self.unit_count);
        let mut ready: Vec<_> = self.units().filter(|unit| peeling.is_source(*unit)).collect();
        while let Some(unit) = ready.pop() {
            peeling.remove(unit);
            front.push(unit);
            let imported = self.dependencies_of(unit).iter().map(|dependency| dependency.imported);
            ready.extend(imported.filter(|unit| peeling.is_source(*unit)));
        }

        // Sinks go last, each before the sinks it imports, which were
        // removed first.
        let mut back = Vec::new();
        let mut ready: Vec<_> = self.units().filter(|unit| peeling.is_sink(*unit)).collect();
        while let Some(unit) = ready.pop() {
            peeling.remove(unit);
            back.push(unit);
            let importers = self.dependencies_on(unit).iter().map(|dependency| dependency.importer);
            ready.extend(importers.filter(|unit| peeling.is_sink(*unit)));
        }

        // The rest go in between, the unit that imports the others most, net
        // of their imports of it, first.
        while let Some(unit) = peeling.most_importing() {
            peeling.remove(unit);
            front.push(unit);
        }

        let mut position: IndexVec<UnitId, usize> = index_vec![0; self.unit_count];
        for (index, unit) in front.into_iter().chain(back.into_iter().rev()).enumerate() {
            position[unit] = index;
        }
        let component = self.components();
        self.dependencies
            .iter()
            .filter(|dependency| {
                position[dependency.imported] < position[dependency.importer]
                    && component[dependency.imported] == component[dependency.importer]
            })
            .copied()
            .collect()
    }

    /// The strongly connected component of each unit, numbered from 0. Two
    /// units are in the same component when each depends on the other,
    /// directly or not, so a dependency is on a cycle exactly when both of
    /// its units are in the same component.
    fn components(&self) -> IndexVec<UnitId, usize> {
        // 1. Finish the units with a depth-first search along their dependencies, in
        //    the order the search leaves them.
        let mut visited: IndexVec<UnitId, bool> = index_vec![false; self.unit_count];
        let mut finished = Vec::with_capacity(self.unit_count);
        let mut stack: Vec<(UnitId, usize)> = Vec::new();
        for start in self.units() {
            if visited[start] {
                continue;
            }
            visited[start] = true;
            stack.push((start, 0));
            // Each entry is a unit and the index of its next dependency to follow.
            while let Some(top) = stack.last_mut() {
                let (unit, next) = *top;
                top.1 += 1;
                match self.dependencies_of(unit).get(next) {
                    Some(dependency) if !visited[dependency.imported] => {
                        visited[dependency.imported] = true;
                        stack.push((dependency.imported, 0));
                    }
                    Some(_) => {}
                    None => {
                        finished.push(unit);
                        stack.pop();
                    }
                }
            }
        }

        // 2. Last finished first, each unit not yet in a component starts one, with the
        //    units that depend on it and aren't in one yet (Kosaraju).
        let mut component: IndexVec<UnitId, Option<usize>> = index_vec![None; self.unit_count];
        let mut count = 0;
        let mut members = Vec::new();
        for root in finished.into_iter().rev() {
            if component[root].is_some() {
                continue;
            }
            component[root] = Some(count);
            members.push(root);
            while let Some(unit) = members.pop() {
                for dependency in self.dependencies_on(unit) {
                    if component[dependency.importer].is_none() {
                        component[dependency.importer] = Some(count);
                        members.push(dependency.importer);
                    }
                }
            }
            count += 1;
        }
        component.into_iter().map(|component| component.expect("every unit is finished")).collect()
    }

    /// The shortest path of dependencies from unit `from` to unit `to`, e.g.
    /// to show a cycle through a breaker. Ties go to the units that come
    /// first. The path from a unit to itself is just the unit.
    ///
    /// Panics if `from` or `to` is not a unit.
    pub fn find_path(&self, from: UnitId, to: UnitId) -> Option<Vec<UnitId>> {
        assert!(
            from.index() < self.unit_count && to.index() < self.unit_count,
            "units {from:?} and {to:?} are not both among the {} units",
            self.unit_count
        );
        // The unit each unit was first reached from. `from` is its own, so
        // that it is never reached again.
        let mut parents: IndexVec<UnitId, Option<UnitId>> = index_vec![None; self.unit_count];
        parents[from] = Some(from);
        let mut queue = VecDeque::from([from]);
        while let Some(unit) = queue.pop_front() {
            if unit == to {
                let mut path: Vec<_> =
                    iter::successors(Some(to), |&unit| parents[unit].filter(|_| unit != from))
                        .collect();
                path.reverse();
                return Some(path);
            }
            for dependency in self.dependencies_of(unit) {
                let parent = &mut parents[dependency.imported];
                if parent.is_none() {
                    *parent = Some(unit);
                    queue.push_back(dependency.imported);
                }
            }
        }
        None
    }

    /// Every unit, in order.
    fn units(&self) -> impl Iterator<Item = UnitId> {
        (0..self.unit_count).map(UnitId::from_usize)
    }

    /// The dependencies of `unit` on other units, sorted by imported unit.
    fn dependencies_of(&self, unit: UnitId) -> &[UnitDependency] {
        let start = self.dependencies.partition_point(|dependency| dependency.importer < unit);
        let end = self.dependencies.partition_point(|dependency| dependency.importer <= unit);
        &self.dependencies[start..end]
    }

    /// The dependencies of other units on `unit`, sorted by importer.
    fn dependencies_on(&self, unit: UnitId) -> &[UnitDependency] {
        let start = self.by_imported.partition_point(|dependency| dependency.imported < unit);
        let end = self.by_imported.partition_point(|dependency| dependency.imported <= unit);
        &self.by_imported[start..end]
    }
}

/// The units left while [`UnitGraph::nominate_cycle_breakers`] puts them in
/// order, with the weighted degrees among them.
///
/// The ordering takes the units out one at a time, and each choice looks only
/// at the dependencies between the units still left: a unit is a source when
/// no unit left imports it, a sink when it imports no unit left, and
/// otherwise ranks by its out-weight minus its in-weight. Rather than recount
/// the weights for every choice, `Peeling` keeps them up to date:
/// [`remove`](Self::remove) takes a unit's dependencies off the in-weights of
/// the units it imports, and its importers' dependencies on it off their
/// out-weights.
///
/// For example, take a cycle where `a` imports `b` three times, `b` imports
/// `c` twice and `c` imports `a` once:
///
/// | unit | out | in | out - in |
/// | ---- | --- | -- | -------- |
/// | `a`  | 3   | 1  | 2        |
/// | `b`  | 2   | 3  | -1       |
/// | `c`  | 1   | 2  | -1       |
///
/// No unit is a source or a sink, so [`most_importing`](Self::most_importing)
/// picks `a`. Removing it takes `a -> b` off `b`'s in-weight and `c -> a` off
/// `c`'s out-weight, which leaves `b` at 2 - 0 and `c` at 0 - 2, so `b` goes
/// next and `c` last. In the order `a`, `b`, `c`, only `c -> a` points
/// backwards, so the lightest dependency of the cycle is its breaker.
///
/// Removing a unit can also make new sources and sinks. If `x` imports `y`,
/// `y` imports `z`, and nothing imports `x`, then `x` is the only source at
/// first. Removing it leaves `y` with an in-weight of 0, so `y` is a source
/// next, then `z`. Sinks peel off the other end of a chain in the same way.
struct Peeling<'u> {
    /// The unit graph.
    units: &'u UnitGraph,

    /// Whether each unit is left.
    left: IndexVec<UnitId, bool>,

    /// The total weight of each unit's dependencies on the units left, while
    /// it is left.
    out_weight: IndexVec<UnitId, i64>,

    /// The total weight of the dependencies on each unit from the units left,
    /// while it is left.
    in_weight: IndexVec<UnitId, i64>,
}

impl<'u> Peeling<'u> {
    /// Start with every unit left.
    fn new(units: &'u UnitGraph) -> Self {
        let mut out_weight = index_vec![0; units.unit_count];
        let mut in_weight = index_vec![0; units.unit_count];
        for dependency in &units.dependencies {
            out_weight[dependency.importer] += i64::from(dependency.weight);
            in_weight[dependency.imported] += i64::from(dependency.weight);
        }
        Self { units, left: index_vec![true; units.unit_count], out_weight, in_weight }
    }

    /// Whether `unit` is left and no unit left imports it. Every weight is
    /// at least 1, so a weight of 0 means no dependencies.
    fn is_source(&self, unit: UnitId) -> bool {
        self.left[unit] && self.in_weight[unit] == 0
    }

    /// Whether `unit` is left and imports no unit left.
    fn is_sink(&self, unit: UnitId) -> bool {
        self.left[unit] && self.out_weight[unit] == 0
    }

    /// The unit left with the highest weighted out-degree minus in-degree,
    /// the first one on ties, or `None` when no unit is left.
    fn most_importing(&self) -> Option<UnitId> {
        // `min_by_key` keeps the first of equal keys.
        self.left
            .indices()
            .filter(|unit| self.left[*unit])
            .min_by_key(|unit| Reverse(self.out_weight[*unit] - self.in_weight[*unit]))
    }

    /// Remove `unit`, and take its dependencies off the weights of the units
    /// at their other ends.
    fn remove(&mut self, unit: UnitId) {
        self.left[unit] = false;
        for dependency in self.units.dependencies_of(unit) {
            self.in_weight[dependency.imported] -= i64::from(dependency.weight);
        }
        for dependency in self.units.dependencies_on(unit) {
            self.out_weight[dependency.importer] -= i64::from(dependency.weight);
        }
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use super::*;
    use crate::{
        Graph, GraphBuilder, ImportKind,
        testing::{detail, graph_of, id, in_both_orders, members},
    };

    /// The unit graph of the members of the nodes called `names`.
    fn units_of(graph: &Graph, names: &[&str]) -> UnitGraph {
        let units: Vec<_> = names.iter().map(|name| members(graph, name)).collect();
        UnitGraph::build(&GraphView::new(graph), &units)
    }

    /// The `(importer, imported)` units of `dependencies`, as indices.
    fn pairs(dependencies: &[UnitDependency]) -> Vec<(usize, usize)> {
        dependencies
            .iter()
            .map(|dependency| (dependency.importer.index(), dependency.imported.index()))
            .collect()
    }

    /// A dependency between the units at indices `importer` and `imported`.
    fn dependency(importer: usize, imported: usize, weight: u32) -> UnitDependency {
        UnitDependency { importer: UnitId::new(importer), imported: UnitId::new(imported), weight }
    }

    /// The path between the units at indices `from` and `to`, as indices.
    fn path(units: &UnitGraph, from: usize, to: usize) -> Option<Vec<usize>> {
        let path = units.find_path(UnitId::new(from), UnitId::new(to))?;
        Some(path.into_iter().map(UnitId::index).collect())
    }

    #[test]
    fn units_aggregate_the_imports_between_their_members() {
        let graph = graph_of(&[
            ("a/x.ts", "b/y.ts"),
            ("a/x.ts", "b/z.ts"),
            ("a/w.ts", "b/y.ts"),
            ("a/w.ts", "a/x.ts"),
            ("b/y.ts", "c/v.ts"),
            ("b/y.ts", "npm:react"),
            ("c/v.ts", "outside.ts"),
        ]);
        let units = units_of(&graph, &["a", "b", "c"]);
        assert_eq!(units.dependencies(), [dependency(0, 1, 3), dependency(1, 2, 1)]);

        let [a, b, c] = [0, 1, 2].map(UnitId::new);
        assert_eq!(pairs(units.dependencies_of(a)), [(0, 1)]);
        assert_eq!(pairs(units.dependencies_of(b)), [(1, 2)]);
        assert!(units.dependencies_of(c).is_empty());
        assert!(units.dependencies_on(a).is_empty());
        assert_eq!(pairs(units.dependencies_on(b)), [(0, 1)]);
        assert_eq!(pairs(units.dependencies_on(c)), [(1, 2)]);
    }

    #[test]
    fn weights_count_pairs_of_members_rather_than_statements() {
        let mut builder = GraphBuilder::new();
        let [x, y, z] = ["a/x.ts", "a/y.ts", "b/z.ts"].map(|path| builder.add_file(path).unwrap());
        for line in 1..30 {
            builder.add_import(x, z, detail(line, ImportKind::Value));
        }
        builder.add_import(y, z, detail(1, ImportKind::Type));
        builder.add_import(z, x, detail(1, ImportKind::Value));
        let graph = builder.finish();

        let units = units_of(&graph, &["a", "b"]);
        assert_eq!(units.dependencies(), [dependency(0, 1, 2), dependency(1, 0, 1),]);
        assert_eq!(units.nominate_cycle_breakers(), [dependency(1, 0, 1)]);
    }

    #[test]
    fn units_follow_the_imports_of_the_view() {
        let graph = graph_of(&[("a/x.ts", "b/y.ts"), ("b/y.ts", "a/x.ts")]);
        let [x, y] = ["a/x.ts", "b/y.ts"].map(|name| id(&graph, name));
        let units = [members(&graph, "a"), members(&graph, "b")];
        let view = GraphView::with_excluded(&graph, [(y, x)].into_iter().collect());
        let unit_graph = UnitGraph::build(&view, &units);
        assert_eq!(pairs(unit_graph.dependencies()), [(0, 1)]);
        assert!(unit_graph.nominate_cycle_breakers().is_empty());
    }

    #[test]
    fn acyclic_units_have_no_breakers() {
        let graph = graph_of(&[
            ("a/x.ts", "b/y.ts"),
            ("b/y.ts", "c/z.ts"),
            ("a/x.ts", "c/z.ts"),
            ("d/w.ts", "b/y.ts"),
        ]);
        assert!(units_of(&graph, &["a", "b", "c", "d"]).nominate_cycle_breakers().is_empty());
        assert!(units_of(&graph, &["a"]).nominate_cycle_breakers().is_empty());
        assert!(UnitGraph::default().nominate_cycle_breakers().is_empty());
    }

    #[test]
    fn breaks_the_lightest_dependency_of_a_cycle() {
        // blue -> green (3 imports), green -> red (2), red -> blue (1).
        let graph = graph_of(&[
            ("blue/1.ts", "green/1.ts"),
            ("blue/2.ts", "green/1.ts"),
            ("blue/3.ts", "green/2.ts"),
            ("green/1.ts", "red/1.ts"),
            ("green/2.ts", "red/1.ts"),
            ("red/1.ts", "blue/1.ts"),
        ]);
        let units = units_of(&graph, &["blue", "green", "red"]);
        assert_eq!(units.nominate_cycle_breakers(), [dependency(2, 0, 1)]);
        assert_eq!(path(&units, 0, 2), Some(vec![0, 1, 2]));
    }

    #[test]
    fn a_two_way_dependency_breaks_its_lighter_side() {
        let graph = graph_of(&[("a/x.ts", "b/y.ts"), ("a/z.ts", "b/y.ts"), ("b/y.ts", "a/x.ts")]);
        assert_eq!(pairs(&units_of(&graph, &["a", "b"]).nominate_cycle_breakers()), [(1, 0)]);

        let graph = graph_of(&[("a/x.ts", "b/y.ts"), ("b/y.ts", "a/x.ts"), ("b/z.ts", "a/x.ts")]);
        assert_eq!(pairs(&units_of(&graph, &["a", "b"]).nominate_cycle_breakers()), [(0, 1)]);
    }

    #[test]
    fn ties_are_broken_by_unit_order() {
        // Every unit of the ring has the same degrees, so `a.ts` goes first
        // and `f.ts -> a.ts` points backwards, whatever order the imports
        // were added in.
        let imports = [
            ("a.ts", "b.ts"),
            ("b.ts", "c.ts"),
            ("c.ts", "d.ts"),
            ("d.ts", "e.ts"),
            ("e.ts", "f.ts"),
            ("f.ts", "a.ts"),
        ];
        let breakers = in_both_orders(&imports, |graph| {
            let units = units_of(graph, &["a.ts", "b.ts", "c.ts", "d.ts", "e.ts", "f.ts"]);
            pairs(&units.nominate_cycle_breakers())
        });
        assert_eq!(breakers, [(5, 0)]);
    }

    #[test]
    fn sources_and_sinks_around_a_cycle_are_never_breakers() {
        let graph = graph_of(&[
            // Sources.
            ("a.ts", "b.ts"),
            ("b.ts", "c.ts"),
            ("c.ts", "d.ts"),
            // The cycle.
            ("d.ts", "e.ts"),
            ("e.ts", "f.ts"),
            ("f.ts", "d.ts"),
            // Sinks.
            ("f.ts", "g.ts"),
            ("g.ts", "h.ts"),
            ("h.ts", "i.ts"),
        ]);
        let names = ["a.ts", "b.ts", "c.ts", "d.ts", "e.ts", "f.ts", "g.ts", "h.ts", "i.ts"];
        let units = units_of(&graph, &names);
        assert_eq!(pairs(&units.nominate_cycle_breakers()), [(5, 3)]);
    }

    #[test]
    fn every_cycle_gets_a_breaker() {
        let graph = graph_of(&[
            ("a/x.ts", "b/x.ts"),
            ("b/x.ts", "a/x.ts"),
            ("a/x.ts", "a/y.ts"),
            ("c/x.ts", "d/x.ts"),
            ("d/x.ts", "e/x.ts"),
            ("e/x.ts", "c/x.ts"),
            ("e/y.ts", "c/x.ts"),
            ("b/x.ts", "c/x.ts"),
        ]);
        let units = units_of(&graph, &["a", "b", "c", "d", "e"]);
        // `b` has the highest out-degree minus in-degree, so it goes first
        // and `a -> b` points backwards.
        let breakers = units.nominate_cycle_breakers();
        assert_eq!(pairs(&breakers), [(0, 1), (3, 4)]);

        // Removing the breakers leaves no cycle.
        let kept = units.dependencies.iter().filter(|dependency| !breakers.contains(dependency));
        let acyclic = UnitGraph::new(units.unit_count, kept.copied().collect());
        assert!(acyclic.nominate_cycle_breakers().is_empty());
    }

    #[test]
    fn every_breaker_is_on_a_cycle() {
        // Two cycles, `b <-> c` and `d <-> e`, and `d -> a -> b` between them.
        // `a` and `d` tie, so `a` goes first and `d -> a` points backwards,
        // although it is on no cycle.
        let graph = graph_of(&[
            ("a.ts", "b.ts"),
            ("a.ts", "c.ts"),
            ("b.ts", "c.ts"),
            ("c.ts", "b.ts"),
            ("d.ts", "a.ts"),
            ("d.ts", "e.ts"),
            ("e.ts", "d.ts"),
        ]);
        let units = units_of(&graph, &["a.ts", "b.ts", "c.ts", "d.ts", "e.ts"]);
        let breakers = units.nominate_cycle_breakers();
        assert_eq!(pairs(&breakers), [(2, 1), (4, 3)]);
        for breaker in breakers {
            assert!(units.find_path(breaker.imported, breaker.importer).is_some());
        }
    }

    #[test]
    fn components_group_the_units_that_depend_on_each_other() {
        let graph = graph_of(&[
            ("a.ts", "b.ts"),
            ("b.ts", "a.ts"),
            ("b.ts", "c.ts"),
            ("c.ts", "d.ts"),
            ("d.ts", "e.ts"),
            ("e.ts", "c.ts"),
            ("f.ts", "a.ts"),
        ]);
        let units = units_of(&graph, &["a.ts", "b.ts", "c.ts", "d.ts", "e.ts", "f.ts"]);
        let component = units.components();
        assert_eq!(component[UnitId::new(0)], component[UnitId::new(1)]);
        assert_eq!(component[UnitId::new(2)], component[UnitId::new(3)]);
        assert_eq!(component[UnitId::new(2)], component[UnitId::new(4)]);
        let distinct: FxHashSet<_> = component.iter().collect();
        assert_eq!(distinct.len(), 3);
    }

    #[test]
    fn unit_paths_are_shortest_and_prefer_earlier_units() {
        let graph = graph_of(&[
            ("a.ts", "c.ts"),
            ("a.ts", "b.ts"),
            ("b.ts", "d.ts"),
            ("c.ts", "d.ts"),
            ("d.ts", "e.ts"),
            ("a.ts", "e.ts"),
            ("e.ts", "a.ts"),
        ]);
        let units = units_of(&graph, &["a.ts", "b.ts", "c.ts", "d.ts", "e.ts"]);
        assert_eq!(path(&units, 0, 3), Some(vec![0, 1, 3]));
        assert_eq!(path(&units, 0, 4), Some(vec![0, 4]));
        assert_eq!(path(&units, 1, 0), Some(vec![1, 3, 4, 0]));
        assert_eq!(path(&units, 2, 2), Some(vec![2]));
        assert_eq!(path(&units, 3, 1), Some(vec![3, 4, 0, 1]));

        let graph = graph_of(&[("a.ts", "b.ts")]);
        let units = units_of(&graph, &["a.ts", "b.ts"]);
        assert_eq!(path(&units, 1, 0), None);
    }

    #[test]
    #[should_panic(expected = "not both among the 2 units")]
    fn unit_paths_need_units_at_both_ends() {
        let graph = graph_of(&[("a.ts", "b.ts")]);
        units_of(&graph, &["a.ts", "b.ts"]).find_path(UnitId::new(0), UnitId::new(2));
    }

    #[test]
    fn breakers_do_not_depend_on_the_order_the_graph_was_built_in() {
        let imports = [
            ("pkg/core/a.ts", "pkg/client/b.ts"),
            ("pkg/client/b.ts", "pkg/core/c.ts"),
            ("pkg/client/d.ts", "pkg/ui/e.ts"),
            ("pkg/ui/e.ts", "pkg/core/a.ts"),
            ("pkg/ui/f.ts", "pkg/client/b.ts"),
            ("pkg/core/c.ts", "pkg/ui/f.ts"),
        ];
        let (_, breakers) = in_both_orders(&imports, |graph| {
            let units = units_of(graph, &["pkg/client", "pkg/core", "pkg/ui"]);
            (units.dependencies().to_vec(), units.nominate_cycle_breakers())
        });
        assert!(!breakers.is_empty());
    }

    #[test]
    fn units_depend_on_the_packages_they_import() {
        let graph = graph_of(&[("a/x.go", "b/"), ("b/y.go", "a/")]);
        let units = units_of(&graph, &["a", "b"]);
        assert_eq!(pairs(units.dependencies()), [(0, 1), (1, 0)]);
        assert_eq!(units.nominate_cycle_breakers().len(), 1);
    }

    #[test]
    fn a_package_and_its_files_are_not_a_cycle() {
        let graph =
            graph_of(&[("cmd/main.go", "lib/"), ("lib/x.go", "util/"), ("lib/sub/y.go", "lib/")]);
        // As one unit, and as units apart: nothing links the package to its
        // files as an import, so the package is imported but never imports.
        let units = units_of(&graph, &["cmd", "lib", "util"]);
        assert_eq!(pairs(units.dependencies()), [(0, 1), (1, 2)]);
        assert!(units.nominate_cycle_breakers().is_empty());

        let view = GraphView::new(&graph);
        let apart = [
            NodeSet::single(id(&graph, "lib")),
            NodeSet::single(id(&graph, "lib/x.go")),
            members(&graph, "lib/sub"),
            members(&graph, "util"),
            members(&graph, "cmd"),
        ];
        let units = UnitGraph::build(&view, &apart);
        assert_eq!(pairs(units.dependencies()), [(1, 3), (2, 0), (4, 0)]);
        assert!(units.nominate_cycle_breakers().is_empty());
    }
}
