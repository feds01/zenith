//! Layered architectures, ported from grimp's `higher_order_queries.rs`.
//!
//! An architecture is a list of [`Level`]s from highest to lowest, each with
//! one or more layers. Higher layers may import lower ones, never the
//! reverse. A query on a [`GraphView`] returns every [`PackageDependency`]
//! that breaks the architecture, with the [`Route`]s of imports that make it.

use std::fmt;

use rayon::prelude::*;

use crate::{GraphView, ModuleId, NodeSet, pathfinding::ChainSearch};

/// One level of a layered architecture: one or more layers at the same
/// height.
#[derive(Debug, Clone, Default)]
pub struct Level {
    /// The layers at this level, each the set of its member nodes. Layers
    /// must not overlap. A layer may be empty, e.g. an optional layer that a
    /// container lacks, so that layer indices stay the same in every
    /// container.
    pub layers: Vec<NodeSet>,

    /// Whether the layers at this level may not import each other.
    pub independent: bool,

    /// Whether the level is closed: the levels above it may not import the
    /// levels below it, so they must go through this one. This holds even if
    /// every layer of the level is empty.
    pub closed: bool,
}

/// A layer of an architecture: its level, and its index within the level.
///
/// Refs order by level, then layer, so from the highest layer to the lowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerRef {
    /// The index of the level, where 0 is the highest.
    pub level: usize,

    /// The index of the layer within its level.
    pub layer: usize,
}

/// An illegal dependency of one layer on another, as grimp names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDependency {
    /// The layer that may not import `imported`.
    pub importer: LayerRef,

    /// The layer that `importer` imports.
    pub imported: LayerRef,

    /// The routes of imports from `importer` to `imported`: the direct
    /// imports in ascending order, then the indirect routes in ascending
    /// order of their middles. Never empty.
    pub routes: Vec<Route>,
}

/// A route of imports from one layer to another.
///
/// A direct import has an empty middle, one head and one tail. An indirect
/// route is a chain of nodes outside every layer, the middle, with every
/// member of the importing layer that imports its first node as heads, and
/// every member of the imported layer that its last node imports as tails.
/// Heads and tails are never empty, and are in ascending id order.
#[derive(Clone, PartialEq, Eq)]
pub struct Route {
    /// The heads, then the middle, then the tails, in one allocation.
    nodes: Vec<ModuleId>,

    /// The index in `nodes` where the middle starts.
    middle_start: u32,

    /// The index in `nodes` where the tails start.
    tails_start: u32,
}

// A dependency can have many routes, so a route is kept to one allocation
// and two indices into it, rather than a list each for its heads, middle and
// tails.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Route>() == 32);

impl Route {
    /// The route of a direct `(importer, imported)` import.
    fn direct((importer, imported): (ModuleId, ModuleId)) -> Self {
        Self { nodes: vec![importer, imported], middle_start: 1, tails_start: 1 }
    }

    /// The route from `heads` through `middle` to `tails`.
    fn indirect(
        heads: impl IntoIterator<Item = ModuleId>,
        middle: &[ModuleId],
        tails: impl IntoIterator<Item = ModuleId>,
    ) -> Self {
        let mut nodes: Vec<ModuleId> = heads.into_iter().collect();
        let middle_start = nodes.len();
        nodes.extend_from_slice(middle);
        let tails_start = nodes.len();
        nodes.extend(tails);

        // A route never repeats a node, and ids are `u32`s, so its indices
        // fit in one too.
        let index = |index: usize| u32::try_from(index).unwrap();
        Self { nodes, middle_start: index(middle_start), tails_start: index(tails_start) }
    }

    /// The members of the importing layer that start the route.
    pub fn heads(&self) -> &[ModuleId] {
        &self.nodes[..self.middle_start as usize]
    }

    /// The chain of nodes between the layers, in import order.
    pub fn middle(&self) -> &[ModuleId] {
        &self.nodes[self.middle_start as usize..self.tails_start as usize]
    }

    /// The members of the imported layer that end the route.
    pub fn tails(&self) -> &[ModuleId] {
        &self.nodes[self.tails_start as usize..]
    }
}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Route")
            .field("heads", &self.heads())
            .field("middle", &self.middle())
            .field("tails", &self.tails())
            .finish()
    }
}

impl GraphView<'_> {
    /// Find every dependency that breaks the layered architecture `levels`
    /// (highest first), sorted by importer and then imported layer.
    ///
    /// The illegal dependencies are: a lower layer importing a higher one; a
    /// layer importing a sibling at an independent level; and a layer
    /// importing a lower layer across a closed level. Chains between two
    /// layers may not pass through any other layer, so `low -> mid -> high`
    /// is reported as the two dependencies `low -> mid` and `mid -> high`,
    /// not a third `low -> high`. The pairs of layers are searched in
    /// parallel.
    pub fn find_illegal_dependencies_for_layers(&self, levels: &[Level]) -> Vec<PackageDependency> {
        let mut all_layer_members = NodeSet::new();
        for layer in levels.iter().flat_map(|level| &level.layers) {
            all_layer_members.union_with(layer);
        }

        // A parallel collect keeps the sorted order of the pairs.
        illegal_pairs(levels)
            .into_par_iter()
            .map_init(ChainSearch::default, |search, (importer, imported)| {
                let routes = self.illegal_routes(
                    search,
                    layer(levels, importer),
                    layer(levels, imported),
                    &all_layer_members,
                );
                (!routes.is_empty()).then_some(PackageDependency { importer, imported, routes })
            })
            .flatten_iter()
            .collect()
    }

    /// Like [`Self::find_illegal_dependencies_for_layers`], but only direct
    /// imports count. Every route is a direct import, with an empty middle.
    pub fn find_illegal_direct_dependencies_for_layers(
        &self,
        levels: &[Level],
    ) -> Vec<PackageDependency> {
        illegal_pairs(levels)
            .into_iter()
            .filter_map(|(importer, imported)| {
                let (from, to) = (layer(levels, importer), layer(levels, imported));
                if from.intersects(to) {
                    return None;
                }
                let routes: Vec<Route> = self.direct_imports(from, to).map(Route::direct).collect();
                (!routes.is_empty()).then_some(PackageDependency { importer, imported, routes })
            })
            .collect()
    }

    /// The routes from `from` to `to` that do not pass through any other
    /// layer.
    fn illegal_routes(
        &self,
        search: &mut ChainSearch,
        from: &NodeSet,
        to: &NodeSet,
        all_layer_members: &NodeSet,
    ) -> Vec<Route> {
        // Excluding every layer leaves out the other layers, since the ends
        // of a chain are never excluded.
        let (direct, indirect) = search.find_chains(self, from, to, all_layer_members);
        let mut routes: Vec<Route> = direct.into_iter().map(Route::direct).collect();

        // Chains with the same middle make the same route, since its heads
        // and tails are every import into and out of the middle.
        let mut middles: Vec<&[ModuleId]> =
            indirect.iter().map(|chain| &chain[1..chain.len() - 1]).collect();
        middles.sort_unstable();
        middles.dedup();
        for middle in middles {
            let (first, last) = (middle[0], middle[middle.len() - 1]);
            routes.push(Route::indirect(
                self.importers_of(first).filter(|head| from.contains(*head)),
                middle,
                self.imports_of(last).filter(|tail| to.contains(*tail)),
            ));
        }
        routes
    }
}

/// The members of a layer.
fn layer(levels: &[Level], layer: LayerRef) -> &NodeSet {
    &levels[layer.level].layers[layer.layer]
}

/// The `(importer, imported)` pairs of layers that may not import each
/// other, sorted.
fn illegal_pairs(levels: &[Level]) -> Vec<(LayerRef, LayerRef)> {
    let refs =
        |level: usize| (0..levels[level].layers.len()).map(move |layer| LayerRef { level, layer });

    let mut pairs = Vec::new();
    for (index, level) in levels.iter().enumerate() {
        for imported in refs(index) {
            // Lower layers may not import it.
            for lower in index + 1..levels.len() {
                pairs.extend(refs(lower).map(|importer| (importer, imported)));
            }

            // Its siblings may not import it, if the level is independent.
            if level.independent {
                let siblings = refs(index).filter(|sibling| *sibling != imported);
                pairs.extend(siblings.map(|sibling| (sibling, imported)));
            }

            // Higher layers may not import it across a closed level.
            let mut closed = false;
            for (higher, higher_level) in levels[..index].iter().enumerate().rev() {
                if closed {
                    pairs.extend(refs(higher).map(|importer| (importer, imported)));
                }
                closed |= higher_level.closed;
            }
        }
    }

    // Each rule makes pairs in its own direction, so there are no duplicates.
    pairs.sort_unstable();
    pairs
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        EdgeSet, Graph,
        testing::{graph_of, id, in_both_orders, names, random_imports, seeded_rng},
    };

    /// A level of the members of the nodes called `names`, one layer each.
    fn level(graph: &Graph, names: &[&str], independent: bool, closed: bool) -> Level {
        let layers = names.iter().map(|name| graph.members(id(graph, name))).collect();
        Level { layers, independent, closed }
    }

    /// A level with a single layer, neither independent nor closed.
    fn single(graph: &Graph, name: &str) -> Level {
        level(graph, &[name], false, false)
    }

    /// A level of `count` empty layers.
    fn empty(count: usize, independent: bool, closed: bool) -> Level {
        Level { layers: vec![NodeSet::new(); count], independent, closed }
    }

    /// A ref to the only layer at a level.
    fn at(level: usize) -> LayerRef {
        LayerRef { level, layer: 0 }
    }

    /// A route by name, as `(heads, middle, tails)`.
    type NamedRoute<'g> = (Vec<&'g str>, Vec<&'g str>, Vec<&'g str>);

    /// The dependencies by name: the importer and imported refs, and the
    /// routes.
    fn named<'g>(
        graph: &'g Graph,
        dependencies: &[PackageDependency],
    ) -> Vec<(LayerRef, LayerRef, Vec<NamedRoute<'g>>)> {
        let by_name = |ids: &[ModuleId]| names(graph, ids.iter().copied());
        dependencies
            .iter()
            .map(|dependency| {
                let routes = dependency
                    .routes
                    .iter()
                    .map(|route| {
                        (by_name(route.heads()), by_name(route.middle()), by_name(route.tails()))
                    })
                    .collect();
                (dependency.importer, dependency.imported, routes)
            })
            .collect()
    }

    /// A direct route by name.
    fn direct<'g>(head: &'g str, tail: &'g str) -> NamedRoute<'g> {
        (vec![head], Vec::new(), vec![tail])
    }

    #[test]
    fn lower_layers_may_not_import_higher_ones() {
        let levels = [empty(1, false, false), empty(1, false, false), empty(1, false, false)];
        assert_eq!(illegal_pairs(&levels), [(at(1), at(0)), (at(2), at(0)), (at(2), at(1))]);
    }

    #[test]
    fn independent_siblings_may_not_import_each_other() {
        let a = LayerRef { level: 0, layer: 0 };
        let b = LayerRef { level: 0, layer: 1 };
        assert_eq!(illegal_pairs(&[empty(2, true, false)]), [(a, b), (b, a)]);
        assert!(illegal_pairs(&[empty(2, false, false)]).is_empty());

        // Every layer below may not import either sibling.
        let low = LayerRef { level: 1, layer: 0 };
        assert_eq!(
            illegal_pairs(&[empty(2, false, false), empty(1, true, false)]),
            [(low, a), (low, b)]
        );
    }

    #[test]
    fn closed_levels_may_not_be_bypassed() {
        let levels = [empty(1, false, false), empty(1, false, true), empty(1, false, false)];
        assert_eq!(
            illegal_pairs(&levels),
            [(at(0), at(2)), (at(1), at(0)), (at(2), at(0)), (at(2), at(1))]
        );

        // A closed level holds back every level above it from every level
        // below it. Closing the top or bottom level changes nothing.
        let levels = [
            empty(1, false, true),
            empty(1, false, false),
            empty(1, false, true),
            empty(1, false, false),
            empty(1, false, true),
        ];
        let downwards: Vec<_> = illegal_pairs(&levels)
            .into_iter()
            .filter(|(importer, imported)| importer < imported)
            .collect();
        assert_eq!(downwards, [(at(0), at(3)), (at(0), at(4)), (at(1), at(3)), (at(1), at(4))]);
    }

    #[test]
    fn chains_through_other_layers_are_reported_layer_by_layer() {
        let graph = graph_of(&[
            ("low/a.ts", "mid/b.ts"),
            ("mid/b.ts", "high/c.ts"),
            ("low/d.ts", "utils/e.ts"),
            ("utils/e.ts", "high/f.ts"),
            ("high/g.ts", "low/a.ts"),
        ]);
        let view = GraphView::new(&graph);
        let levels = [single(&graph, "high"), single(&graph, "mid"), single(&graph, "low")];
        let dependencies = view.find_illegal_dependencies_for_layers(&levels);
        assert_eq!(
            named(&graph, &dependencies),
            [
                (at(1), at(0), vec![direct("mid/b.ts", "high/c.ts")]),
                (at(2), at(0), vec![(vec!["low/d.ts"], vec!["utils/e.ts"], vec!["high/f.ts"])]),
                (at(2), at(1), vec![direct("low/a.ts", "mid/b.ts")]),
            ]
        );
    }

    /// grimp's legal graph for its layer tests, as files.
    const LEGAL: [(&str, &str); 5] = [
        ("high/green.ts", "medium/orange.ts"),
        ("high/green.ts", "low/white/gamma.ts"),
        ("medium/orange.ts", "low/white.ts"),
        ("high/blue.ts", "utils.ts"),
        ("utils.ts", "medium/red.ts"),
    ];

    /// Check grimp's high, medium and low layers on its legal graph, plus
    /// `imports`.
    fn check_with(imports: &[(&'static str, &'static str)]) -> (Graph, Vec<PackageDependency>) {
        let graph = graph_of(&[&LEGAL[..], imports].concat());
        let view = GraphView::new(&graph);
        let levels = [single(&graph, "high"), single(&graph, "medium"), single(&graph, "low")];
        let dependencies = view.find_illegal_dependencies_for_layers(&levels);
        (graph, dependencies)
    }

    #[test]
    fn legal_imports_break_nothing() {
        let (_, dependencies) = check_with(&[]);
        assert!(dependencies.is_empty());
    }

    #[test]
    fn reports_each_illegal_pair_of_layers() {
        let (graph, dependencies) = check_with(&[
            ("low/white.ts", "medium/orange/beta.ts"),
            ("medium/orange.ts", "high/green.ts"),
        ]);
        assert_eq!(
            named(&graph, &dependencies),
            [
                (at(1), at(0), vec![direct("medium/orange.ts", "high/green.ts")]),
                (at(2), at(1), vec![direct("low/white.ts", "medium/orange/beta.ts")]),
            ]
        );
    }

    #[test]
    fn routes_with_the_same_ends_are_reported_separately() {
        let (graph, dependencies) = check_with(&[
            ("medium/orange.ts", "tungsten.ts"),
            ("tungsten.ts", "copper.ts"),
            ("copper.ts", "high/green.ts"),
            ("medium/orange.ts", "gold/delta.ts"),
            ("gold/delta.ts", "high/green.ts"),
        ]);
        let ends = |middle| (vec!["medium/orange.ts"], middle, vec!["high/green.ts"]);
        assert_eq!(
            named(&graph, &dependencies),
            [(
                at(1),
                at(0),
                vec![ends(vec!["gold/delta.ts"]), ends(vec!["tungsten.ts", "copper.ts"])]
            )]
        );
    }

    #[test]
    fn routes_collect_every_head_and_tail_of_their_middle() {
        let (graph, dependencies) = check_with(&[
            ("medium/orange.ts", "tungsten.ts"),
            ("tungsten.ts", "copper.ts"),
            ("copper.ts", "high/green.ts"),
            // Extra heads.
            ("medium/orange/beta.ts", "tungsten.ts"),
            ("medium/red.ts", "tungsten.ts"),
            // Extra tails.
            ("copper.ts", "high/index.ts"),
            ("copper.ts", "high/yellow/alpha.ts"),
        ]);
        assert_eq!(
            named(&graph, &dependencies),
            [(
                at(1),
                at(0),
                vec![(
                    vec!["medium/orange.ts", "medium/orange/beta.ts", "medium/red.ts"],
                    vec!["tungsten.ts", "copper.ts"],
                    vec!["high/green.ts", "high/index.ts", "high/yellow/alpha.ts"],
                )]
            )]
        );
    }

    #[test]
    fn finds_equal_routes_but_not_longer_forks() {
        let graph = graph_of(&[
            ("low/blue.ts", "a.ts"),
            ("a.ts", "b.ts"),
            ("b.ts", "high/green.ts"),
            ("low/blue.ts", "c.ts"),
            ("c.ts", "d.ts"),
            ("d.ts", "high/green.ts"),
            // A longer fork of the first route reuses its first import.
            ("a.ts", "e.ts"),
            ("e.ts", "f.ts"),
            ("f.ts", "high/green.ts"),
        ]);
        let view = GraphView::new(&graph);
        let levels = [single(&graph, "high"), single(&graph, "low")];
        let ends = |middle| (vec!["low/blue.ts"], middle, vec!["high/green.ts"]);
        assert_eq!(
            named(&graph, &view.find_illegal_dependencies_for_layers(&levels)),
            [(at(1), at(0), vec![ends(vec!["a.ts", "b.ts"]), ends(vec!["c.ts", "d.ts"])])]
        );
    }

    #[test]
    fn chains_through_independent_siblings_are_not_included() {
        let graph = graph_of(&[("blue/alpha.ts", "green/beta.ts"), ("yellow.ts", "blue/alpha.ts")]);
        let view = GraphView::new(&graph);
        let siblings = level(&graph, &["blue", "green", "yellow.ts"], true, false);
        let dependencies = view.find_illegal_dependencies_for_layers(&[siblings]);
        let [blue, green, yellow] = [0, 1, 2].map(|layer| LayerRef { level: 0, layer });
        assert_eq!(
            named(&graph, &dependencies),
            [
                (blue, green, vec![direct("blue/alpha.ts", "green/beta.ts")]),
                (yellow, blue, vec![direct("yellow.ts", "blue/alpha.ts")]),
            ]
        );
    }

    #[test]
    fn non_independent_siblings_may_import_each_other() {
        let graph = graph_of(&[
            ("types/a.ts", "config/b.ts"),
            ("config/b.ts", "types/a.ts"),
            ("config/b.ts", "app/c.ts"),
        ]);
        let view = GraphView::new(&graph);
        let levels = [single(&graph, "app"), level(&graph, &["types", "config"], false, false)];
        let config = LayerRef { level: 1, layer: 1 };
        assert_eq!(
            named(&graph, &view.find_illegal_dependencies_for_layers(&levels)),
            [(config, at(0), vec![direct("config/b.ts", "app/c.ts")])]
        );
    }

    #[test]
    fn chains_across_a_closed_level_must_pass_through_it() {
        let graph = graph_of(&[
            ("high/a.ts", "mid/b.ts"),
            ("mid/b.ts", "low/c.ts"),
            ("high/d.ts", "low/e.ts"),
            ("high/f.ts", "utils.ts"),
            ("utils.ts", "low/c.ts"),
        ]);
        let view = GraphView::new(&graph);
        let open = [single(&graph, "high"), single(&graph, "mid"), single(&graph, "low")];
        assert!(view.find_illegal_dependencies_for_layers(&open).is_empty());

        let closed =
            [single(&graph, "high"), level(&graph, &["mid"], false, true), single(&graph, "low")];
        assert_eq!(
            named(&graph, &view.find_illegal_dependencies_for_layers(&closed)),
            [(
                at(0),
                at(2),
                vec![
                    direct("high/d.ts", "low/e.ts"),
                    (vec!["high/f.ts"], vec!["utils.ts"], vec!["low/c.ts"]),
                ]
            )]
        );
        assert_eq!(
            named(&graph, &view.find_illegal_direct_dependencies_for_layers(&closed)),
            [(at(0), at(2), vec![direct("high/d.ts", "low/e.ts")])]
        );
    }

    #[test]
    fn missing_layers_are_skipped_but_still_close_their_level() {
        let graph = graph_of(&[("low/a.ts", "high/b.ts"), ("high/b.ts", "low/c.ts")]);
        let view = GraphView::new(&graph);

        // An optional middle layer that is missing.
        let levels = [single(&graph, "high"), empty(1, false, false), single(&graph, "low")];
        assert_eq!(
            named(&graph, &view.find_illegal_dependencies_for_layers(&levels)),
            [(at(2), at(0), vec![direct("low/a.ts", "high/b.ts")])]
        );

        // A missing sibling keeps the indices of the others.
        let mut siblings = level(&graph, &["high"], true, false);
        siblings.layers.insert(0, NodeSet::new());
        let levels = [siblings, single(&graph, "low")];
        let high = LayerRef { level: 0, layer: 1 };
        assert_eq!(
            named(&graph, &view.find_illegal_dependencies_for_layers(&levels)),
            [(at(1), high, vec![direct("low/a.ts", "high/b.ts")])]
        );

        // A closed level still closes when its layers are missing, as in
        // grimp.
        let levels = [single(&graph, "high"), empty(1, false, true), single(&graph, "low")];
        assert_eq!(
            named(&graph, &view.find_illegal_dependencies_for_layers(&levels)),
            [
                (at(0), at(2), vec![direct("high/b.ts", "low/c.ts")]),
                (at(2), at(0), vec![direct("low/a.ts", "high/b.ts")]),
            ]
        );
    }

    #[test]
    fn containers_are_checked_separately() {
        // grimp's test of several containers, each checked with its own
        // levels. Chains may pass through another container.
        let graph = graph_of(&[
            // Legal imports.
            ("one/high/yellow.ts", "one/low/white/gamma.ts"),
            ("one/high/brown.ts", "one/utils.ts"),
            ("one/utils.ts", "one/medium/pink.ts"),
            ("two/medium/index.ts", "two/low/index.ts"),
            ("two/medium/purple.ts", "two/low/index.ts"),
            ("two/medium/index.ts", "two/low/black/epsilon.ts"),
            // Lower layers importing higher ones across containers are legal.
            ("two/medium/pink.ts", "one/high/blue.ts"),
            // Illegal imports.
            ("one/low/white.ts", "one/high/green.ts"),
            ("one/low/white.ts", "two/medium/pink.ts"),
            ("two/medium/pink.ts", "one/high/green.ts"),
            ("one/low/white.ts", "one/high/brown.ts"),
            ("two/medium/pink/delta.ts", "two/high/yellow/gamma.ts"),
        ]);
        let view = GraphView::new(&graph);
        let levels_in = |container: &str| {
            ["high", "medium", "low"].map(|layer| single(&graph, &format!("{container}/{layer}")))
        };

        assert_eq!(
            named(&graph, &view.find_illegal_dependencies_for_layers(&levels_in("one"))),
            [(
                at(2),
                at(0),
                vec![
                    direct("one/low/white.ts", "one/high/brown.ts"),
                    direct("one/low/white.ts", "one/high/green.ts"),
                    (
                        vec!["one/low/white.ts"],
                        vec!["two/medium/pink.ts"],
                        vec!["one/high/blue.ts", "one/high/green.ts"],
                    ),
                ]
            )]
        );
        assert_eq!(
            named(&graph, &view.find_illegal_dependencies_for_layers(&levels_in("two"))),
            [(at(1), at(0), vec![direct("two/medium/pink/delta.ts", "two/high/yellow/gamma.ts")])]
        );
    }

    #[test]
    fn direct_only_layers_ignore_indirect_chains() {
        let graph = graph_of(&[
            ("low/a.ts", "utils.ts"),
            ("utils.ts", "high/b.ts"),
            ("low/c.ts", "high/b.ts"),
            ("low/c.ts", "high/d.ts"),
            ("low/e.ts", "high/b.ts"),
        ]);
        let view = GraphView::new(&graph);
        let levels = [single(&graph, "high"), single(&graph, "low")];
        assert_eq!(
            named(&graph, &view.find_illegal_direct_dependencies_for_layers(&levels)),
            [(
                at(1),
                at(0),
                vec![
                    direct("low/c.ts", "high/b.ts"),
                    direct("low/c.ts", "high/d.ts"),
                    direct("low/e.ts", "high/b.ts"),
                ]
            )]
        );

        let [c, b] = ["low/c.ts", "high/b.ts"].map(|name| id(&graph, name));
        let view = GraphView::with_excluded(&graph, [(c, b)].into_iter().collect());
        let dependencies = view.find_illegal_direct_dependencies_for_layers(&levels);
        assert_eq!(dependencies[0].routes.len(), 2);
    }

    #[test]
    fn overlapping_layers_have_no_dependencies() {
        let graph = graph_of(&[("web/a.ts", "web/lib/b.ts"), ("web/lib/b.ts", "web/c.ts")]);
        let view = GraphView::new(&graph);
        let levels = [single(&graph, "web"), single(&graph, "web/lib")];
        assert!(view.find_illegal_dependencies_for_layers(&levels).is_empty());
        assert!(view.find_illegal_direct_dependencies_for_layers(&levels).is_empty());
    }

    #[test]
    fn dependencies_do_not_depend_on_the_order_the_graph_was_built_in() {
        let imports = [
            ("pkg/core/a.ts", "pkg/client/b.ts"),
            ("pkg/client/b.ts", "pkg/ui/c.ts"),
            ("pkg/core/d.ts", "lib/e.ts"),
            ("lib/e.ts", "pkg/ui/f.ts"),
            ("lib/e.ts", "pkg/ui/c.ts"),
            ("pkg/core/g.ts", "lib/e.ts"),
            ("pkg/ui/c.ts", "pkg/forms/h.ts"),
            ("pkg/forms/h.ts", "pkg/ui/f.ts"),
        ];
        let dependencies = in_both_orders(&imports, |graph| {
            let view = GraphView::new(graph);
            let levels = [
                level(graph, &["pkg/ui", "pkg/forms"], true, false),
                single(graph, "pkg/client"),
                single(graph, "pkg/core"),
            ];
            format!("{:?}", named(graph, &view.find_illegal_dependencies_for_layers(&levels)))
        });
        assert!(dependencies.contains("lib/e.ts"));
    }

    /// grimp's `find_illegal_dependencies_for_layers`, step by step: skip the
    /// pairs without any chain, then search with every other layer excluded,
    /// and make a route of each direct chain and of each distinct middle,
    /// with heads and tails found by looking at every member of the layers.
    fn reference_dependencies(view: &GraphView, levels: &[Level]) -> Vec<PackageDependency> {
        let mut all_layer_members = NodeSet::new();
        for layer in levels.iter().flat_map(|level| &level.layers) {
            all_layer_members.union_with(layer);
        }
        let mut dependencies = Vec::new();
        for (importer, imported) in illegal_pairs(levels) {
            let (from, to) = (layer(levels, importer), layer(levels, imported));
            if view.find_shortest_chain(from, to, &NodeSet::new(), &EdgeSet::new()).is_none() {
                continue;
            }
            let mut other_layers = all_layer_members.clone();
            other_layers.difference_with(from);
            other_layers.difference_with(to);

            let mut routes = Vec::new();
            let mut middles = BTreeSet::new();
            for chain in view.find_shortest_chains(from, to, &other_layers) {
                if let [head, tail] = chain[..] {
                    routes.push(Route::direct((head, tail)));
                } else {
                    middles.insert(chain[1..chain.len() - 1].to_vec());
                }
            }
            for middle in middles {
                let (first, last) = (middle[0], middle[middle.len() - 1]);
                let heads = from.iter().filter(|head| view.imports_of(*head).any(|m| m == first));
                let tails = to.iter().filter(|tail| view.imports_of(last).any(|m| m == *tail));
                routes.push(Route::indirect(heads, &middle, tails));
            }
            if !routes.is_empty() {
                dependencies.push(PackageDependency { importer, imported, routes });
            }
        }
        dependencies
    }

    #[test]
    fn dependencies_match_grimp_on_generated_graphs() {
        // Excluding every layer, rather than every layer but the two being
        // searched, and skipping grimp's check for any chain at all, must
        // give the same dependencies.
        let mut random = seeded_rng();
        let mut files: Vec<String> = Vec::new();
        for layer in ["high", "left", "right", "low"] {
            files.extend(["a", "b", "c"].map(|file| format!("{layer}/{file}.ts")));
        }
        files.extend((0..4).map(|index| format!("lib/{index}.ts")));

        let (mut with_dependencies, mut indirect) = (0, 0);
        for _ in 0..300 {
            let count = random.usize(..40);
            let imports = random_imports(&mut random, &files, count);
            if imports.is_empty() {
                continue;
            }
            let graph = graph_of(&imports);
            let view = GraphView::new(&graph);
            // A layer whose files are never imported is missing.
            let members = |name| graph.id(name).map_or_else(NodeSet::new, |id| graph.members(id));
            let levels = [
                Level { layers: vec![members("high")], independent: false, closed: random.bool() },
                Level {
                    layers: vec![members("left"), members("right")],
                    independent: random.bool(),
                    closed: random.bool(),
                },
                Level { layers: vec![members("low")], independent: false, closed: random.bool() },
            ];

            let dependencies = view.find_illegal_dependencies_for_layers(&levels);
            assert_eq!(dependencies, reference_dependencies(&view, &levels), "{imports:?}");
            with_dependencies += usize::from(!dependencies.is_empty());
            indirect += dependencies
                .iter()
                .flat_map(|dependency| &dependency.routes)
                .filter(|route| !route.middle().is_empty())
                .count();
        }
        assert!(with_dependencies > 50 && indirect > 50, "{with_dependencies} {indirect}");
    }
}
