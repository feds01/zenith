//! How the dependencies between the parts of a module changed from one graph
//! to another, such as from a project at one git commit to the same project
//! at another.

use std::{collections::BTreeMap, fmt};

use rustc_hash::FxHashSet;
use serde::Serialize;
use zenith_diagnostics::message;

use crate::{
    ChildDependency, Graph, GraphView, ModuleId, NodeSet, UnitGraph, overview::display_name,
};

/// A module, and the dependencies between the nodes some levels below it
/// that were added and removed from a base graph to a head graph, matching
/// the nodes by name.
///
/// It is shown as the changed dependencies, `+` for added and `-` for
/// removed, through [`fmt::Display`], and serialises as `{"module", "added":
/// [{"from", "to", "imports"}], "removed": [...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependencyDiff<'g> {
    /// How many levels below the module the nodes are: 1 for its children.
    #[serde(skip)]
    depth: usize,

    /// The name of the module, with `.` for the root.
    pub module: &'g str,

    /// The dependencies of the head that the base doesn't have, sorted by
    /// importing then imported node.
    pub added: Vec<ChildDependency<'g>>,

    /// The dependencies of the base that the head doesn't have, named as in
    /// the base, and sorted by importing then imported node.
    pub removed: Vec<ChildDependency<'g>>,
}

impl<'g> DependencyDiff<'g> {
    /// The diff of each module of the nodes `base_roots` in `base` and
    /// `head_roots` in `head`, matched by name: of the dependencies between
    /// the nodes `depth` levels below it. A module of only one side is
    /// compared with nothing, so all of its dependencies were added, or all
    /// removed.
    ///
    /// `follow` gives a name in the base its name in the head, such as the
    /// new name of a renamed file, so that its dependencies are not both
    /// removed and added. The diffs are sorted by module.
    pub fn between(
        (base, base_roots): (&GraphView<'g>, &[ModuleId]),
        (head, head_roots): (&GraphView<'g>, &[ModuleId]),
        depth: usize,
        follow: impl Fn(&'g str) -> &'g str,
    ) -> Vec<Self> {
        let mut modules: BTreeMap<&'g str, (Vec<_>, Vec<_>)> = BTreeMap::new();
        for root in base_roots {
            let module = follow(display_name(base.graph(), *root));
            modules.entry(module).or_default().0 = base.dependencies_below(*root, depth);
        }
        for root in head_roots {
            let module = display_name(head.graph(), *root);
            modules.entry(module).or_default().1 = head.dependencies_below(*root, depth);
        }
        modules
            .into_iter()
            .map(|(module, (base, head))| Self::new(module, depth, &base, &head, &follow))
            .collect()
    }

    /// The diff of `module` from the dependencies `base` to the dependencies
    /// `head`, with `follow` giving the names of the base in the head.
    fn new(
        module: &'g str,
        depth: usize,
        base: &[ChildDependency<'g>],
        head: &[ChildDependency<'g>],
        follow: impl Fn(&'g str) -> &'g str,
    ) -> Self {
        let followed =
            |dependency: &ChildDependency<'g>| (follow(dependency.from), follow(dependency.to));
        let in_base: FxHashSet<_> = base.iter().map(followed).collect();
        let in_head: FxHashSet<_> =
            head.iter().map(|dependency| (dependency.from, dependency.to)).collect();
        let added = head
            .iter()
            .filter(|dependency| !in_base.contains(&(dependency.from, dependency.to)))
            .copied()
            .collect();
        let removed = base
            .iter()
            .filter(|dependency| !in_head.contains(&followed(dependency)))
            .copied()
            .collect();
        Self { depth, module, added, removed }
    }

    /// Whether no dependency was added or removed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The changed dependencies, in name order, as the lines of a diff.
impl fmt::Display for DependencyDiff<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { depth, module, added, removed } = self;
        if self.is_empty() {
            return writeln!(f, "{}", message!(graph_diff_unchanged, module, depth));
        }
        writeln!(f, "{}", message!(graph_diff_changed, module, depth))?;
        let mut lines: Vec<_> = removed.iter().map(|dependency| ('-', dependency)).collect();
        lines.extend(added.iter().map(|dependency| ('+', dependency)));
        lines.sort_by_key(|(_, dependency)| (dependency.from, dependency.to));
        for (sign, dependency) in lines {
            writeln!(f, "  {sign} {dependency}")?;
        }
        Ok(())
    }
}

impl<'g> GraphView<'g> {
    /// The dependencies in this view between the nodes `depth` levels below
    /// the node `id`, or less deep where the tree ends sooner, so the
    /// children of `id` at depth 1. They are sorted by importing then
    /// imported node.
    pub fn dependencies_below(&self, id: ModuleId, depth: usize) -> Vec<ChildDependency<'g>> {
        let graph = self.graph();
        let mut nodes = Vec::new();
        graph.nodes_below(id, depth, &mut nodes);
        let units: Vec<NodeSet> = nodes.iter().map(|node| graph.members(*node)).collect();
        let mut dependencies: Vec<_> = UnitGraph::build(self, &units)
            .dependencies()
            .iter()
            .map(|dependency| ChildDependency {
                from: graph.name(nodes[dependency.importer.index()]),
                to: graph.name(nodes[dependency.imported.index()]),
                imports: dependency.weight,
            })
            .collect();
        dependencies.sort_unstable_by_key(|dependency| (dependency.from, dependency.to));
        dependencies
    }
}

impl Graph {
    /// Add the nodes `depth` levels below `id` to `nodes`, or the files and
    /// packages where the tree ends sooner.
    fn nodes_below(&self, id: ModuleId, depth: usize, nodes: &mut Vec<ModuleId>) {
        for child in self.children(id) {
            match depth > 1 && !self.children(*child).is_empty() {
                true => self.nodes_below(*child, depth - 1, nodes),
                false => nodes.push(*child),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{graph_of, id};

    /// `web/src/app.ts` imports `lib/a.ts`, which imports `types.ts`.
    fn base() -> Graph {
        graph_of(&[
            ("web/src/app.ts", "web/src/lib/a.ts"),
            ("web/src/lib/a.ts", "web/src/types.ts"),
            ("web/src/lib/b.ts", "web/src/lib/a.ts"),
        ])
    }

    /// [`base`], but `app.ts` imports `types.ts` too, and `lib/a.ts` no
    /// longer does.
    fn head() -> Graph {
        graph_of(&[
            ("web/src/app.ts", "web/src/lib/a.ts"),
            ("web/src/app.ts", "web/src/types.ts"),
            ("web/src/lib/b.ts", "web/src/lib/a.ts"),
            ("web/src/lib/a.ts", "npm:react"),
        ])
    }

    /// The diffs from `base` to `head` under the nodes called `name`, `depth`
    /// levels below them, following no renames.
    fn diff<'g>(
        base: &'g Graph,
        head: &'g Graph,
        name: &str,
        depth: usize,
    ) -> Vec<DependencyDiff<'g>> {
        let (base_view, head_view) = (GraphView::new(base), GraphView::new(head));
        let base_roots: Vec<_> = base.id(name).into_iter().collect();
        let head_roots: Vec<_> = head.id(name).into_iter().collect();
        DependencyDiff::between(
            (&base_view, &base_roots),
            (&head_view, &head_roots),
            depth,
            |name| name,
        )
    }

    #[test]
    fn finds_the_dependencies_below_a_node() {
        let graph = base();
        let view = GraphView::new(&graph);
        let children = view.dependencies_below(id(&graph, "web/src"), 1);
        assert_eq!(
            children,
            [
                ChildDependency { from: "web/src/app.ts", to: "web/src/lib", imports: 1 },
                ChildDependency { from: "web/src/lib", to: "web/src/types.ts", imports: 1 },
            ]
        );
        let grandchildren: Vec<_> = view
            .dependencies_below(id(&graph, "web/src"), 2)
            .iter()
            .map(|dependency| (dependency.from, dependency.to))
            .collect();
        assert_eq!(
            grandchildren,
            [
                ("web/src/app.ts", "web/src/lib/a.ts"),
                ("web/src/lib/a.ts", "web/src/types.ts"),
                ("web/src/lib/b.ts", "web/src/lib/a.ts"),
            ]
        );
    }

    #[test]
    fn finds_the_added_and_removed_dependencies() {
        let (base, head) = (base(), head());
        let [web] = diff(&base, &head, "web/src", 1).try_into().unwrap();
        assert_eq!(web.module, "web/src");
        assert_eq!(
            web.added,
            [ChildDependency { from: "web/src/app.ts", to: "web/src/types.ts", imports: 1 }]
        );
        assert_eq!(
            web.removed,
            [ChildDependency { from: "web/src/lib", to: "web/src/types.ts", imports: 1 }]
        );

        // The imports inside `lib` and of `npm:react` are not between its children.
        let [lib] = diff(&base, &head, "web/src/lib", 1).try_into().unwrap();
        assert!(lib.is_empty());
    }

    #[test]
    fn follows_renamed_files() {
        let base = graph_of(&[("src/old.ts", "src/a.ts"), ("src/b.ts", "src/old.ts")]);
        let head = graph_of(&[("src/new.ts", "src/a.ts"), ("src/new.ts", "src/b.ts")]);
        let (base_view, head_view) = (GraphView::new(&base), GraphView::new(&head));
        let (base_roots, head_roots) = ([id(&base, "src")], [id(&head, "src")]);
        let follow = |name| if name == "src/old.ts" { "src/new.ts" } else { name };
        let diffs = DependencyDiff::between(
            (&base_view, &base_roots),
            (&head_view, &head_roots),
            1,
            follow,
        );

        // `old.ts -> a.ts` is still there, as `new.ts -> a.ts`; the base names the
        // removed dependency as it was.
        let [src] = diffs.try_into().unwrap();
        assert_eq!(src.added, [ChildDependency { from: "src/new.ts", to: "src/b.ts", imports: 1 }]);
        assert_eq!(
            src.removed,
            [ChildDependency { from: "src/b.ts", to: "src/old.ts", imports: 1 }]
        );
    }

    #[test]
    fn compares_a_module_of_one_side_with_nothing() {
        let (base, head) = (graph_of(&[("api/a.ts", "api/b.ts")]), base());
        let (base_view, head_view) = (GraphView::new(&base), GraphView::new(&head));
        let base_roots = [id(&base, "api")];
        let head_roots = [id(&head, "web/src")];
        let diffs = DependencyDiff::between(
            (&base_view, &base_roots),
            (&head_view, &head_roots),
            1,
            |name| name,
        );
        let modules: Vec<_> =
            diffs.iter().map(|diff| (diff.module, diff.added.len(), diff.removed.len())).collect();
        assert_eq!(modules, [("api", 0, 1), ("web/src", 2, 0)]);
    }

    #[test]
    fn shows_the_changes_as_the_lines_of_a_diff() {
        let (base, head) = (base(), head());
        let expected = [
            "dependencies between the children of `web/src` that changed:",
            "  + web/src/app.ts -> web/src/types.ts (1 import)",
            "  - web/src/lib -> web/src/types.ts (1 import)",
        ];
        assert_eq!(diff(&base, &head, "web/src", 1)[0].to_string(), expected.join("\n") + "\n");

        let expected = [
            "dependencies between the nodes 2 levels below `web/src` that changed:",
            "  + web/src/app.ts -> web/src/types.ts (1 import)",
            "  - web/src/lib/a.ts -> web/src/types.ts (1 import)",
        ];
        assert_eq!(diff(&base, &head, "web/src", 2)[0].to_string(), expected.join("\n") + "\n");

        let unchanged = "no dependencies between the children of `.` changed\n";
        assert_eq!(diff(&base, &head, "", 1)[0].to_string(), unchanged);
    }

    #[test]
    fn serialises_the_added_and_removed_dependencies() {
        let (base, head) = (base(), head());
        let json = serde_json::to_value(diff(&base, &head, "web/src", 1)).unwrap();
        assert_eq!(
            json,
            serde_json::json!([{
                "module": "web/src",
                "added": [{ "from": "web/src/app.ts", "to": "web/src/types.ts", "imports": 1 }],
                "removed": [{ "from": "web/src/lib", "to": "web/src/types.ts", "imports": 1 }],
            }])
        );
    }
}
