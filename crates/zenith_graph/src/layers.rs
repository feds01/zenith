//! Layered architecture queries, ported from grimp's
//! `higher_order_queries.rs`.

use rayon::prelude::*;
use rustc_hash::FxHashSet;

use crate::{GraphView, ModuleId, NodeSet, find_shortest_chain, find_shortest_chains};

/// One level in a layered architecture: one or more layers that sit at the
/// same height.
#[derive(Debug, Clone)]
pub struct Level {
    /// The layers at this level, each a set of member nodes.
    pub layers: Vec<NodeSet>,
    /// Whether the layers at this level may not import each other.
    pub independent: bool,
    /// Whether the level is closed: levels above it may not import levels
    /// below it, i.e. they must go through this level.
    pub closed: bool,
}

/// Identifies a layer: its level, and its index within the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerRef {
    pub level: usize,
    pub layer: usize,
}

/// An illegal dependency of one layer on another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDependency {
    /// The layer that may not import `imported`.
    pub importer: LayerRef,
    /// The layer being illegally imported.
    pub imported: LayerRef,
    /// Every distinct route from `importer` to `imported`.
    pub routes: Vec<Route>,
}

/// A route of imports between two layers. A direct import has an empty
/// `middle`, one head and one tail. An indirect route lists every member of
/// the importing layer that imports the first middle module (`heads`), and
/// every member of the imported layer that the last middle module imports
/// (`tails`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Route {
    pub heads: Vec<ModuleId>,
    pub middle: Vec<ModuleId>,
    pub tails: Vec<ModuleId>,
}

/// Find every dependency that breaks the layered architecture `levels`,
/// ordered from highest to lowest.
///
/// The illegal pairs are: a lower layer importing a higher one; a layer
/// importing a sibling in an independent level; and a layer importing a
/// lower layer across a closed level. Chains between a pair may not pass
/// through any other layer, so `low -> mid -> high` is reported as the two
/// dependencies `low -> mid` and `mid -> high`.
pub fn find_illegal_dependencies_for_layers(
    view: &GraphView,
    levels: &[Level],
) -> Vec<PackageDependency> {
    let mut all_layer_members = NodeSet::new();
    for level in levels {
        for layer in &level.layers {
            all_layer_members.union_with(layer);
        }
    }

    let layer = |r: LayerRef| &levels[r.level].layers[r.layer];
    let mut permutations: Vec<_> = illegal_permutations(levels).into_iter().collect();
    permutations.sort();

    let mut dependencies: Vec<PackageDependency> = permutations
        .into_par_iter()
        .filter_map(|(importer, imported)| {
            find_illegal_dependency(view, layer(importer), layer(imported), &all_layer_members)
                .map(|routes| PackageDependency { importer, imported, routes })
        })
        .collect();

    dependencies.sort_by_key(|dependency| (dependency.importer, dependency.imported));
    dependencies
}

/// Like [find_illegal_dependencies_for_layers], but only considering direct
/// imports. Every route has an empty `middle`, one head and one tail.
pub fn find_illegal_direct_dependencies_for_layers(
    view: &GraphView,
    levels: &[Level],
) -> Vec<PackageDependency> {
    let layer = |r: LayerRef| &levels[r.level].layers[r.layer];
    let mut permutations: Vec<_> = illegal_permutations(levels).into_iter().collect();
    permutations.sort();

    permutations
        .into_iter()
        .filter_map(|(importer, imported)| {
            let (from, to) = (layer(importer), layer(imported));
            if from.intersects(to) {
                return None;
            }
            let routes: Vec<Route> =
                from.iter()
                    .flat_map(|head| {
                        view.imports_of(head).filter(|tail| to.contains(*tail)).map(move |tail| {
                            Route { heads: vec![head], middle: vec![], tails: vec![tail] }
                        })
                    })
                    .collect();
            (!routes.is_empty()).then_some(PackageDependency { importer, imported, routes })
        })
        .collect()
}

/// The `(importer, imported)` layer pairs that are not allowed.
fn illegal_permutations(levels: &[Level]) -> FxHashSet<(LayerRef, LayerRef)> {
    let mut permutations = FxHashSet::default();

    for (index, level) in levels.iter().enumerate() {
        for layer in 0..level.layers.len() {
            let module = LayerRef { level: index, layer };

            // Should not be imported by lower layers.
            for (lower_index, lower_level) in levels.iter().enumerate().skip(index + 1) {
                for lower_layer in 0..lower_level.layers.len() {
                    permutations
                        .insert((LayerRef { level: lower_index, layer: lower_layer }, module));
                }
            }

            // Should not import siblings, if the level is independent.
            if level.independent {
                for sibling in 0..level.layers.len() {
                    if sibling != layer {
                        permutations.insert((module, LayerRef { level: index, layer: sibling }));
                    }
                }
            }

            // Should not be imported by higher layers if there is a closed
            // layer in between.
            let mut closed = false;
            for higher_index in (0..index).rev() {
                if closed {
                    for higher_layer in 0..levels[higher_index].layers.len() {
                        permutations.insert((
                            LayerRef { level: higher_index, layer: higher_layer },
                            module,
                        ));
                    }
                }
                closed |= levels[higher_index].closed;
            }
        }
    }

    permutations
}

fn find_illegal_dependency(
    view: &GraphView,
    from_layer: &NodeSet,
    to_layer: &NodeSet,
    all_layer_members: &NodeSet,
) -> Option<Vec<Route>> {
    if from_layer.intersects(to_layer) {
        return None;
    }

    // Shortcut the detailed search when there is no chain at all.
    let no_exclusions = NodeSet::new();
    find_shortest_chain(view, from_layer, to_layer, &no_exclusions, &Default::default())?;

    // Disallow chains via other layers.
    let mut excluded = all_layer_members.clone();
    excluded.difference_with(from_layer);
    excluded.difference_with(to_layer);

    let chains = find_shortest_chains(view, from_layer, to_layer, &excluded);

    let mut direct = Vec::new();
    let mut middles = Vec::new();
    for chain in chains {
        if chain.len() == 2 {
            direct.push(Route { heads: vec![chain[0]], middle: vec![], tails: vec![chain[1]] });
        } else {
            middles.push(chain[1..chain.len() - 1].to_vec());
        }
    }
    middles.sort();
    middles.dedup();

    let mut routes = direct;
    routes.sort();
    for middle in middles {
        let first = middle[0];
        let last = *middle.last().unwrap();
        let heads = view.importers_of(first).filter(|head| from_layer.contains(*head)).collect();
        let tails = view.imports_of(last).filter(|tail| to_layer.contains(*tail)).collect();
        routes.push(Route { heads, middle, tails });
    }

    (!routes.is_empty()).then_some(routes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{graph_of, id};

    fn level(layers: Vec<NodeSet>, independent: bool, closed: bool) -> Level {
        Level { layers, independent, closed }
    }

    fn single(graph: &crate::Graph, name: &str) -> NodeSet {
        graph.members(id(graph, name))
    }

    #[test]
    fn permutations_for_simple_layers() {
        let levels = vec![
            level(vec![NodeSet::new()], false, false),
            level(vec![NodeSet::new()], false, false),
            level(vec![NodeSet::new()], false, false),
        ];
        let r = |level| LayerRef { level, layer: 0 };
        let expected: FxHashSet<_> =
            [(r(2), r(1)), (r(2), r(0)), (r(1), r(0))].into_iter().collect();
        assert_eq!(illegal_permutations(&levels), expected);
    }

    #[test]
    fn permutations_for_independent_layer() {
        let levels = vec![level(vec![NodeSet::new(), NodeSet::new()], true, false)];
        let a = LayerRef { level: 0, layer: 0 };
        let b = LayerRef { level: 0, layer: 1 };
        let expected: FxHashSet<_> = [(a, b), (b, a)].into_iter().collect();
        assert_eq!(illegal_permutations(&levels), expected);
    }

    #[test]
    fn permutations_for_closed_layer() {
        let levels = vec![
            level(vec![NodeSet::new()], false, false),
            level(vec![NodeSet::new()], false, true),
            level(vec![NodeSet::new()], false, false),
        ];
        let r = |level| LayerRef { level, layer: 0 };
        let expected: FxHashSet<_> =
            [(r(2), r(1)), (r(2), r(0)), (r(1), r(0)), (r(0), r(2))].into_iter().collect();
        assert_eq!(illegal_permutations(&levels), expected);
    }

    #[test]
    fn reports_chains_via_other_layers_separately() {
        let graph = graph_of(&[
            ("low/a.ts", "mid/b.ts"),
            ("mid/b.ts", "high/c.ts"),
            ("low/d.ts", "utils/e.ts"),
            ("utils/e.ts", "high/f.ts"),
            ("high/g.ts", "low/a.ts"),
        ]);
        let view = GraphView::new(&graph);
        let levels = vec![
            level(vec![single(&graph, "high")], false, false),
            level(vec![single(&graph, "mid")], false, false),
            level(vec![single(&graph, "low")], false, false),
        ];
        let dependencies = find_illegal_dependencies_for_layers(&view, &levels);
        let summary: Vec<_> = dependencies
            .iter()
            .map(|d| (d.importer.level, d.imported.level, d.routes.len()))
            .collect();
        // mid -> high (direct); low -> high (via utils, not via mid);
        // low -> mid (direct).
        assert_eq!(summary, vec![(1, 0, 1), (2, 0, 1), (2, 1, 1)]);

        let low_to_high = &dependencies[1];
        let route = &low_to_high.routes[0];
        assert_eq!(route.heads, vec![id(&graph, "low/d.ts")]);
        assert_eq!(route.middle, vec![id(&graph, "utils/e.ts")]);
        assert_eq!(route.tails, vec![id(&graph, "high/f.ts")]);
    }
}
