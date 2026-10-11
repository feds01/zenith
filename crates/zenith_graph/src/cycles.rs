//! Dependency cycles between units, and the dependencies to remove to break
//! them, ported from grimp's `cycle_breakers.rs`.
//!
//! A [`UnitGraph`] aggregates the direct imports between the members of a
//! list of units into weighted dependencies between the units, which are
//! referred to by their index in that list.

use std::{collections::VecDeque, iter, mem};

use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::{GraphView, NodeSet};

/// A dependency of one unit on another.
///
/// Dependencies order by importer, then imported unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitDependency {
    /// The index of the importing unit.
    pub importer: usize,

    /// The index of the imported unit.
    pub imported: usize,

    /// The number of imports that make the dependency: the `(importer,
    /// imported)` pairs of members, however many statements make each pair.
    pub weight: usize,
}

/// The dependencies between a list of disjoint units, aggregated from the
/// direct imports between their members.
#[derive(Debug, Clone, Default)]
pub struct UnitGraph {
    /// The number of units.
    unit_count: usize,

    /// Every dependency, sorted by importer then imported unit.
    dependencies: Vec<UnitDependency>,
}

impl UnitGraph {
    /// Aggregate the imports between `units`, which must not overlap.
    ///
    /// Imports within a unit, and imports of nodes outside every unit, are
    /// not dependencies. The order of `units` breaks ties in
    /// [`nominate_cycle_breakers`].
    pub fn build(view: &GraphView, units: &[NodeSet]) -> Self {
        // Only the members matter, so look them up in a map of their own
        // rather than a table of every node in the graph.
        let member_count = units.iter().map(NodeSet::len).sum();
        let mut unit_of = FxHashMap::with_capacity_and_hasher(member_count, FxBuildHasher);
        for (index, unit) in units.iter().enumerate() {
            for member in unit.iter() {
                let previous = unit_of.insert(member, index);
                debug_assert!(previous.is_none(), "units must not overlap");
            }
        }

        // Count the imports of one unit at a time into `weights`, indexed by
        // imported unit, so the dependencies come out in order.
        let mut weights = vec![0; units.len()];
        let mut targets = Vec::new();
        let mut dependencies = Vec::new();
        for (importer, unit) in units.iter().enumerate() {
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

        UnitGraph { unit_count: units.len(), dependencies }
    }

    /// Every dependency, sorted by importer then imported unit.
    pub fn dependencies(&self) -> &[UnitDependency] {
        &self.dependencies
    }

    /// The dependencies of `unit` on other units, sorted by imported unit.
    fn dependencies_of(&self, unit: usize) -> &[UnitDependency] {
        let start = self.dependencies.partition_point(|dependency| dependency.importer < unit);
        let end = self.dependencies.partition_point(|dependency| dependency.importer <= unit);
        &self.dependencies[start..end]
    }
}

/// Nominate the dependencies to remove so that the units become acyclic,
/// sorted by importer then imported unit. Empty when they already are.
///
/// This is grimp's weighted variant of the Eades–Lin–Smyth heuristic for
/// the minimum feedback arc set. The units are put in order: first the
/// sources, which nothing left imports; last the sinks, which import nothing
/// left; and in between the rest, picking the unit with the highest weighted
/// out-degree minus in-degree each time. Every dependency that points
/// backwards in that order is a breaker.
///
/// Ties go to the unit that comes first, so pass the units in alphabetical
/// order of their labels to break ties alphabetically, as grimp does.
pub fn nominate_cycle_breakers(units: &UnitGraph) -> Vec<UnitDependency> {
    let unit_count = units.unit_count;
    let mut peeling = Peeling::new(units);

    // Sources go first. Removing one can make the units it imports sources.
    let mut front = Vec::with_capacity(unit_count);
    let mut ready: Vec<usize> = (0..unit_count).filter(|unit| peeling.is_source(*unit)).collect();
    while let Some(unit) = ready.pop() {
        peeling.remove(unit);
        front.push(unit);
        ready.extend(
            units
                .dependencies_of(unit)
                .iter()
                .map(|dependency| dependency.imported)
                .filter(|imported| peeling.is_source(*imported)),
        );
    }

    // Sinks go last, each before the sinks it imports, which were removed
    // first.
    let mut back = Vec::new();
    let mut ready: Vec<usize> = (0..unit_count).filter(|unit| peeling.is_sink(*unit)).collect();
    while let Some(unit) = ready.pop() {
        peeling.remove(unit);
        back.push(unit);
        ready.extend(
            peeling.importers[unit]
                .iter()
                .map(|index| units.dependencies[*index].importer)
                .filter(|importer| peeling.is_sink(*importer)),
        );
    }

    // The rest go in between, the unit that imports the others most, net of
    // their imports of it, first.
    while let Some(unit) = peeling.most_importing() {
        peeling.remove(unit);
        front.push(unit);
    }

    let mut position = vec![0; unit_count];
    for (index, unit) in front.iter().chain(back.iter().rev()).enumerate() {
        position[*unit] = index;
    }
    units
        .dependencies
        .iter()
        .filter(|dependency| position[dependency.imported] < position[dependency.importer])
        .copied()
        .collect()
}

/// The shortest path of dependencies from unit `from` to unit `to`, as unit
/// indices from `from` to `to`, e.g. to show a cycle through a breaker. Ties
/// go to the units that come first. The path from a unit to itself is just
/// the unit.
///
/// Panics if `from` or `to` is not the index of a unit.
pub fn find_unit_path(units: &UnitGraph, from: usize, to: usize) -> Option<Vec<usize>> {
    assert!(
        from < units.unit_count && to < units.unit_count,
        "units {from} and {to} are not both among the {} units",
        units.unit_count
    );
    // The unit each unit was first reached from. `from` is its own, so that
    // it is never reached again.
    let mut parents: Vec<Option<usize>> = vec![None; units.unit_count];
    parents[from] = Some(from);
    let mut queue = VecDeque::from([from]);
    while let Some(unit) = queue.pop_front() {
        if unit == to {
            let mut path: Vec<_> =
                iter::successors(Some(to), |&node| parents[node].filter(|_| node != from))
                    .collect();
            path.reverse();
            return Some(path);
        }
        for dependency in units.dependencies_of(unit) {
            let parent = &mut parents[dependency.imported];
            if parent.is_none() {
                *parent = Some(unit);
                queue.push_back(dependency.imported);
            }
        }
    }
    None
}

/// The units left while [`nominate_cycle_breakers`] orders them, with the
/// weighted degrees among them.
struct Peeling<'u> {
    /// The unit graph.
    units: &'u UnitGraph,

    /// The positions in `units.dependencies` of each unit's importers.
    importers: Vec<Vec<usize>>,

    /// Whether each unit is left.
    left: Vec<bool>,

    /// The total weight of each unit's dependencies on units left.
    out_weight: Vec<usize>,

    /// The total weight of the dependencies on each unit from units left.
    in_weight: Vec<usize>,
}

impl<'u> Peeling<'u> {
    /// Start with every unit left.
    fn new(units: &'u UnitGraph) -> Self {
        let unit_count = units.unit_count;
        let mut importers = vec![Vec::new(); unit_count];
        let mut out_weight = vec![0; unit_count];
        let mut in_weight = vec![0; unit_count];
        for (index, dependency) in units.dependencies.iter().enumerate() {
            importers[dependency.imported].push(index);
            out_weight[dependency.importer] += dependency.weight;
            in_weight[dependency.imported] += dependency.weight;
        }
        Self { units, importers, left: vec![true; unit_count], out_weight, in_weight }
    }

    /// Whether `unit` is left and no unit left imports it. Every weight is
    /// at least 1, so a weight of 0 means no dependencies.
    fn is_source(&self, unit: usize) -> bool {
        self.left[unit] && self.in_weight[unit] == 0
    }

    /// Whether `unit` is left and imports no unit left.
    fn is_sink(&self, unit: usize) -> bool {
        self.left[unit] && self.out_weight[unit] == 0
    }

    /// The unit left with the highest weighted out-degree minus in-degree,
    /// the first one on ties, or `None` when no unit is left.
    fn most_importing(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for unit in (0..self.units.unit_count).filter(|unit| self.left[*unit]) {
            // `out(unit) - in(unit) > out(best) - in(best)`, without signs.
            if best.is_none_or(|best| {
                self.out_weight[unit] + self.in_weight[best]
                    > self.out_weight[best] + self.in_weight[unit]
            }) {
                best = Some(unit);
            }
        }
        best
    }

    /// Remove `unit`, and its dependencies from the degrees of the units
    /// left.
    fn remove(&mut self, unit: usize) {
        self.left[unit] = false;
        for dependency in self.units.dependencies_of(unit) {
            if self.left[dependency.imported] {
                self.in_weight[dependency.imported] -= dependency.weight;
            }
        }
        for index in &self.importers[unit] {
            let dependency = &self.units.dependencies[*index];
            if self.left[dependency.importer] {
                self.out_weight[dependency.importer] -= dependency.weight;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Graph, GraphBuilder, ImportKind,
        testing::{detail, graph_of, id, in_both_orders},
    };

    /// The unit graph of the members of the nodes called `names`.
    fn units_of(graph: &Graph, names: &[&str]) -> UnitGraph {
        let units: Vec<_> = names.iter().map(|name| graph.members(id(graph, name))).collect();
        UnitGraph::build(&GraphView::new(graph), &units)
    }

    /// The `(importer, imported)` units of `dependencies`.
    fn pairs(dependencies: &[UnitDependency]) -> Vec<(usize, usize)> {
        dependencies.iter().map(|dependency| (dependency.importer, dependency.imported)).collect()
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
        assert_eq!(
            units.dependencies(),
            [
                UnitDependency { importer: 0, imported: 1, weight: 3 },
                UnitDependency { importer: 1, imported: 2, weight: 1 },
            ]
        );
        assert_eq!(pairs(units.dependencies_of(0)), [(0, 1)]);
        assert_eq!(pairs(units.dependencies_of(1)), [(1, 2)]);
        assert!(units.dependencies_of(2).is_empty());
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
        assert_eq!(
            units.dependencies(),
            [
                UnitDependency { importer: 0, imported: 1, weight: 2 },
                UnitDependency { importer: 1, imported: 0, weight: 1 },
            ]
        );
        assert_eq!(
            nominate_cycle_breakers(&units),
            [UnitDependency { importer: 1, imported: 0, weight: 1 }]
        );
    }

    #[test]
    fn units_follow_the_imports_of_the_view() {
        let graph = graph_of(&[("a/x.ts", "b/y.ts"), ("b/y.ts", "a/x.ts")]);
        let [x, y] = ["a/x.ts", "b/y.ts"].map(|name| id(&graph, name));
        let units = [graph.members(id(&graph, "a")), graph.members(id(&graph, "b"))];
        let view = GraphView::with_excluded(&graph, [(y, x)].into_iter().collect());
        let unit_graph = UnitGraph::build(&view, &units);
        assert_eq!(pairs(unit_graph.dependencies()), [(0, 1)]);
        assert!(nominate_cycle_breakers(&unit_graph).is_empty());
    }

    #[test]
    fn acyclic_units_have_no_breakers() {
        let graph = graph_of(&[
            ("a/x.ts", "b/y.ts"),
            ("b/y.ts", "c/z.ts"),
            ("a/x.ts", "c/z.ts"),
            ("d/w.ts", "b/y.ts"),
        ]);
        assert!(nominate_cycle_breakers(&units_of(&graph, &["a", "b", "c", "d"])).is_empty());
        assert!(nominate_cycle_breakers(&units_of(&graph, &["a"])).is_empty());
        assert!(nominate_cycle_breakers(&UnitGraph::default()).is_empty());
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
        assert_eq!(
            nominate_cycle_breakers(&units),
            [UnitDependency { importer: 2, imported: 0, weight: 1 }]
        );
        assert_eq!(find_unit_path(&units, 0, 2), Some(vec![0, 1, 2]));
    }

    #[test]
    fn a_two_way_dependency_breaks_its_lighter_side() {
        let graph = graph_of(&[("a/x.ts", "b/y.ts"), ("a/z.ts", "b/y.ts"), ("b/y.ts", "a/x.ts")]);
        assert_eq!(pairs(&nominate_cycle_breakers(&units_of(&graph, &["a", "b"]))), [(1, 0)]);

        let graph = graph_of(&[("a/x.ts", "b/y.ts"), ("b/y.ts", "a/x.ts"), ("b/z.ts", "a/x.ts")]);
        assert_eq!(pairs(&nominate_cycle_breakers(&units_of(&graph, &["a", "b"]))), [(0, 1)]);
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
            pairs(&nominate_cycle_breakers(&units))
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
        assert_eq!(pairs(&nominate_cycle_breakers(&units)), [(5, 3)]);
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
        let breakers = nominate_cycle_breakers(&units);
        assert_eq!(pairs(&breakers), [(0, 1), (3, 4)]);

        // Removing the breakers leaves no cycle.
        let mut acyclic = units.clone();
        acyclic.dependencies.retain(|dependency| !breakers.contains(dependency));
        assert!(nominate_cycle_breakers(&acyclic).is_empty());
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
        assert_eq!(find_unit_path(&units, 0, 3), Some(vec![0, 1, 3]));
        assert_eq!(find_unit_path(&units, 0, 4), Some(vec![0, 4]));
        assert_eq!(find_unit_path(&units, 1, 0), Some(vec![1, 3, 4, 0]));
        assert_eq!(find_unit_path(&units, 2, 2), Some(vec![2]));
        assert_eq!(find_unit_path(&units, 3, 1), Some(vec![3, 4, 0, 1]));

        let graph = graph_of(&[("a.ts", "b.ts")]);
        let units = units_of(&graph, &["a.ts", "b.ts"]);
        assert_eq!(find_unit_path(&units, 1, 0), None);
    }

    #[test]
    #[should_panic(expected = "not both among the 2 units")]
    fn unit_paths_need_units_at_both_ends() {
        let graph = graph_of(&[("a.ts", "b.ts")]);
        find_unit_path(&units_of(&graph, &["a.ts", "b.ts"]), 0, 2);
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
            (units.dependencies().to_vec(), nominate_cycle_breakers(&units))
        });
        assert!(!breakers.is_empty());
    }
}
