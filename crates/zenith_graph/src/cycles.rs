//! Dependency cycles between units, ported from grimp's `cycle_breakers.rs`.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, VecDeque},
};

use crate::{GraphView, NodeSet};

/// The dependencies between a set of disjoint units, aggregated from the
/// direct imports between their members.
#[derive(Debug, Clone, Default)]
pub struct UnitGraph {
    /// The number of units.
    pub len: usize,
    /// `(importer unit, imported unit)` to the number of direct
    /// `(importer, imported)` module pairs between them.
    pub weights: BTreeMap<(usize, usize), usize>,
}

impl UnitGraph {
    /// Aggregate the imports between `units`. Units must be disjoint;
    /// imports within a unit are not dependencies.
    pub fn build(view: &GraphView, units: &[NodeSet]) -> Self {
        let mut unit_of = vec![u32::MAX; view.graph().len()];
        for (index, unit) in units.iter().enumerate() {
            for member in unit.iter() {
                unit_of[member.index()] = index as u32;
            }
        }

        let mut weights = BTreeMap::new();
        for (index, unit) in units.iter().enumerate() {
            for member in unit.iter() {
                for imported in view.imports_of(member) {
                    let target = unit_of[imported.index()];
                    if target != u32::MAX && target as usize != index {
                        *weights.entry((index, target as usize)).or_insert(0) += 1;
                    }
                }
            }
        }

        UnitGraph { len: units.len(), weights }
    }

    /// The units that `unit` depends on.
    pub fn successors(&self, unit: usize) -> impl Iterator<Item = usize> + '_ {
        self.weights.range((unit, 0)..(unit + 1, 0)).map(|((_, to), _)| *to)
    }
}

/// Nominate dependencies to remove so that the units become acyclic. Returns
/// an empty list when they already are.
///
/// This is grimp's weighted variant of the Eades–Lin–Smyth heuristic for the
/// minimum feedback arc set. Units are ordered (sources first, sinks last, the
/// rest by weighted out-degree minus in-degree, ties broken alphabetically by
/// label), and every dependency pointing backwards in that order is a
/// breaker.
pub fn nominate_cycle_breakers(units: &UnitGraph, labels: &[String]) -> Vec<(usize, usize)> {
    if units.len < 2 {
        return Vec::new();
    }

    let mut imports: BTreeMap<usize, BTreeSet<usize>> =
        (0..units.len).map(|u| (u, BTreeSet::new())).collect();
    let mut importers = imports.clone();
    for (importer, imported) in units.weights.keys() {
        imports.get_mut(importer).unwrap().insert(*imported);
        importers.get_mut(imported).unwrap().insert(*importer);
    }

    let remove = |unit: usize,
                  imports: &mut BTreeMap<usize, BTreeSet<usize>>,
                  importers: &mut BTreeMap<usize, BTreeSet<usize>>| {
        imports.remove(&unit);
        importers.remove(&unit);
        for set in imports.values_mut().chain(importers.values_mut()) {
            set.remove(&unit);
        }
    };

    // Iteratively extract sources.
    let mut sources = Vec::new();
    loop {
        let current: Vec<usize> =
            importers.iter().filter(|(_, from)| from.is_empty()).map(|(unit, _)| *unit).collect();
        if current.is_empty() {
            break;
        }
        for unit in current {
            remove(unit, &mut imports, &mut importers);
            sources.push(unit);
        }
    }

    // Iteratively extract sinks. Later sinks depend on earlier ones, so they
    // go first in the overall order.
    let mut sinks: Vec<usize> = Vec::new();
    loop {
        let current: Vec<usize> =
            imports.iter().filter(|(_, to)| to.is_empty()).map(|(unit, _)| *unit).collect();
        if current.is_empty() {
            break;
        }
        for unit in &current {
            remove(*unit, &mut imports, &mut importers);
        }
        sinks.splice(0..0, current);
    }

    // Order the remaining units by weighted out-degree minus in-degree.
    let mut middle = Vec::new();
    while !imports.is_empty() {
        let degree = |unit: usize| -> isize {
            let out: usize = imports[&unit].iter().map(|to| units.weights[&(unit, *to)]).sum();
            let inn: usize =
                importers[&unit].iter().map(|from| units.weights[&(*from, unit)]).sum();
            out as isize - inn as isize
        };

        let mut best: Option<(usize, isize)> = None;
        for unit in imports.keys().copied() {
            let difference = degree(unit);
            best = match best {
                None => Some((unit, difference)),
                Some((incumbent, highest)) => match difference.cmp(&highest) {
                    Ordering::Greater => Some((unit, difference)),
                    Ordering::Equal if labels[unit] < labels[incumbent] => Some((unit, difference)),
                    _ => Some((incumbent, highest)),
                },
            };
        }

        let (unit, _) = best.unwrap();
        middle.push(unit);
        remove(unit, &mut imports, &mut importers);
    }

    let mut position = vec![0; units.len];
    for (index, unit) in sources.iter().chain(&middle).chain(&sinks).enumerate() {
        position[*unit] = index;
    }

    let mut breakers: Vec<(usize, usize)> = units
        .weights
        .keys()
        .filter(|(importer, imported)| position[*imported] < position[*importer])
        .copied()
        .collect();
    breakers.sort_by(|a, b| (&labels[a.0], &labels[a.1]).cmp(&(&labels[b.0], &labels[b.1])));
    breakers
}

/// The shortest path of dependencies from unit `from` to unit `to`, used to
/// show an example cycle through a cycle breaker.
pub fn find_unit_path(units: &UnitGraph, from: usize, to: usize) -> Option<Vec<usize>> {
    let mut parent = vec![usize::MAX; units.len];
    let mut queue = VecDeque::from([from]);
    parent[from] = from;
    while let Some(unit) = queue.pop_front() {
        if unit == to {
            let mut path = vec![to];
            let mut node = to;
            while node != from {
                node = parent[node];
                path.push(node);
            }
            path.reverse();
            return Some(path);
        }
        for next in units.successors(unit) {
            if parent[next] == usize::MAX {
                parent[next] = unit;
                queue.push_back(next);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{graph_of, id};

    fn units_of(graph: &crate::Graph, names: &[&str]) -> (Vec<NodeSet>, Vec<String>) {
        let units = names.iter().map(|name| graph.members(id(graph, name))).collect();
        (units, names.iter().map(|name| name.to_string()).collect())
    }

    #[test]
    fn acyclic_units_have_no_breakers() {
        let graph = graph_of(&[("a/x.ts", "b/y.ts"), ("b/y.ts", "c/z.ts")]);
        let (units, labels) = units_of(&graph, &["a", "b", "c"]);
        let unit_graph = UnitGraph::build(&GraphView::new(&graph), &units);
        assert!(nominate_cycle_breakers(&unit_graph, &labels).is_empty());
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
        let (units, labels) = units_of(&graph, &["blue", "green", "red"]);
        let unit_graph = UnitGraph::build(&GraphView::new(&graph), &units);
        assert_eq!(unit_graph.weights[&(0, 1)], 3);
        assert_eq!(nominate_cycle_breakers(&unit_graph, &labels), vec![(2, 0)]);
        assert_eq!(find_unit_path(&unit_graph, 0, 2), Some(vec![0, 1, 2]));
    }

    #[test]
    fn two_way_dependency_breaks_one_side() {
        let graph = graph_of(&[("a/x.ts", "b/y.ts"), ("a/z.ts", "b/y.ts"), ("b/y.ts", "a/x.ts")]);
        let (units, labels) = units_of(&graph, &["a", "b"]);
        let unit_graph = UnitGraph::build(&GraphView::new(&graph), &units);
        assert_eq!(nominate_cycle_breakers(&unit_graph, &labels), vec![(1, 0)]);
    }
}
