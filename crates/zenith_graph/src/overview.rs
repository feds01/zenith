//! Overviews of the module tree: a node, its children, and the dependencies
//! between them, for showing the shape of part of a project as a tree, a
//! Graphviz digraph, or JSON.

use std::fmt;

use serde::Serialize;
use zenith_utils::{
    printing::plural,
    tree_writing::{TreeNode, TreeWriter},
};

use crate::{Graph, GraphView, ModuleId, NodeKind, NodeSet, UnitGraph};

/// A node of the module tree, its children, and the dependencies between
/// the children in a [`GraphView`].
///
/// It is shown with [`Self::tree`] or [`Self::dot`], and serialises as
/// `{"module", "children": [{"name", "kind", "files"}], "dependencies":
/// [{"from", "to", "imports"}]}`.
#[derive(Clone, Serialize)]
pub struct ModuleOverview<'g> {
    /// The graph that the node is in.
    #[serde(skip)]
    graph: &'g Graph,

    /// The node.
    #[serde(skip)]
    id: ModuleId,

    /// The name of the node, with `.` for the root.
    module: &'g str,

    /// The children of the node, in name order.
    children: Vec<OverviewChild<'g>>,

    /// The dependencies between the children, sorted by importing then
    /// imported child.
    dependencies: Vec<ChildDependency<'g>>,
}

/// A child of the node of a [`ModuleOverview`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OverviewChild<'g> {
    /// The child.
    #[serde(skip)]
    pub id: ModuleId,

    /// The name of the child.
    pub name: &'g str,

    /// What the child is.
    pub kind: NodeKind,

    /// The number of files in the child, or 1 for a file. A package is not a
    /// file, so it is not counted.
    pub files: usize,
}

/// A dependency between two children of the node of a [`ModuleOverview`],
/// shown as `<from> -> <to> (<n> imports)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChildDependency<'g> {
    /// The name of the importing child.
    pub from: &'g str,

    /// The name of the imported child.
    pub to: &'g str,

    /// The number of imports between their members: the `(importer,
    /// imported)` pairs, however many statements make each pair.
    pub imports: u32,
}

impl fmt::Display for ChildDependency<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {} ({})", self.from, self.to, plural(self.imports as usize, "import"))
    }
}

impl<'g> ModuleOverview<'g> {
    /// The overview of the node `id`, with the dependencies between its
    /// children in `view`.
    pub fn new(view: &GraphView<'g>, id: ModuleId) -> Self {
        let graph = view.graph();
        let children = graph.children(id);
        let units: Vec<NodeSet> = children.iter().map(|child| graph.members(*child)).collect();
        let dependencies = UnitGraph::build(view, &units)
            .dependencies()
            .iter()
            .map(|dependency| ChildDependency {
                from: graph.name(children[dependency.importer.index()]),
                to: graph.name(children[dependency.imported.index()]),
                imports: dependency.weight,
            })
            .collect();
        let children = children
            .iter()
            .zip(&units)
            .map(|(child, members)| {
                let node = graph.node(*child);
                let (name, kind, files) = (node.name(), node.kind(), count_files(graph, members));
                OverviewChild { id: *child, name, kind, files }
            })
            .collect();
        Self { graph, id, module: display_name(graph, id), children, dependencies }
    }

    /// The children of the node, in name order.
    pub fn children(&self) -> &[OverviewChild<'g>] {
        &self.children
    }

    /// The dependencies between the children, sorted by importing then
    /// imported child.
    pub fn dependencies(&self) -> &[ChildDependency<'g>] {
        &self.dependencies
    }

    /// The module tree under the node, `depth` levels deep, followed by the
    /// dependencies between its children.
    pub fn tree(&self, depth: usize) -> OverviewTree<'_, 'g> {
        OverviewTree { overview: self, depth }
    }

    /// A Graphviz digraph of the children and the dependencies between them.
    pub fn dot(&self) -> OverviewDot<'_, 'g> {
        OverviewDot { overview: self }
    }
}

/// Shows a [`ModuleOverview`] as its module tree, followed by the
/// dependencies between the children, through [`fmt::Display`].
pub struct OverviewTree<'a, 'g> {
    /// The overview to show.
    overview: &'a ModuleOverview<'g>,

    /// How many levels of the tree to show.
    depth: usize,
}

impl fmt::Display for OverviewTree<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ModuleOverview { graph, id, module, dependencies, .. } = self.overview;
        write!(f, "{}", TreeWriter::new(&tree_node(graph, *id, self.depth, true)))?;
        if !dependencies.is_empty() {
            writeln!(f, "\ndependencies between the children of `{module}`:")?;
            for dependency in dependencies {
                writeln!(f, "  {dependency}")?;
            }
        }
        Ok(())
    }
}

/// Shows a [`ModuleOverview`] as a Graphviz digraph, through
/// [`fmt::Display`].
pub struct OverviewDot<'a, 'g> {
    /// The overview to show.
    overview: &'a ModuleOverview<'g>,
}

impl fmt::Display for OverviewDot<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ModuleOverview { graph, module, children, dependencies, .. } = self.overview;
        writeln!(f, "digraph {} {{", quote(module))?;
        writeln!(f, "  rankdir=LR;\n  node [shape=box];")?;
        for child in children {
            let base_name = graph.node(child.id).base_name();
            let label = match child.kind {
                NodeKind::Directory => format!("{base_name}/"),
                _ => base_name.to_string(),
            };
            writeln!(f, "  {} [label={}];", quote(child.name), quote(&label))?;
        }
        for ChildDependency { from, to, imports } in dependencies {
            writeln!(f, "  {} -> {} [label=\"{imports}\"];", quote(from), quote(to))?;
        }
        writeln!(f, "}}")
    }
}

/// The tree under `id`, `depth` levels deep. The top node is labelled with
/// its full name, the others with their base name. Directories show how many
/// files they hold.
fn tree_node(graph: &Graph, id: ModuleId, depth: usize, top: bool) -> TreeNode {
    let node = graph.node(id);
    let shown = if top { display_name(graph, id) } else { node.base_name() };
    if node.kind() != NodeKind::Directory {
        return TreeNode::leaf(shown);
    }

    let slash = if id == ModuleId::ROOT { "" } else { "/" };
    let label =
        format!("{shown}{slash} ({})", plural(count_files(graph, &graph.members(id)), "file"));
    if depth == 0 {
        return TreeNode::leaf(label);
    }
    let children =
        graph.children(id).iter().map(|child| tree_node(graph, *child, depth - 1, false));
    TreeNode::branch(label, children.collect())
}

/// The number of files among `members`, which can hold packages too.
fn count_files(graph: &Graph, members: &NodeSet) -> usize {
    members.iter().filter(|member| graph.node(*member).kind() != NodeKind::Directory).count()
}

/// The name of a node, with `.` for the root.
fn display_name(graph: &Graph, id: ModuleId) -> &str {
    if id == ModuleId::ROOT { "." } else { graph.name(id) }
}

/// A Graphviz string.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImportKind, KindSet, testing::graph_of};

    /// `web/src/app.ts` imports `lib/a.ts` and `lib/b.ts`, which import
    /// `types.ts`.
    fn graph() -> Graph {
        graph_of(&[
            ("web/src/app.ts", "web/src/lib/a.ts"),
            ("web/src/app.ts", "web/src/lib/b.ts"),
            ("web/src/lib/a.ts", "web/src/types.ts"),
            ("web/src/lib/b.ts", "web/src/types.ts"),
        ])
    }

    /// The overview of the node called `name`.
    fn overview<'g>(graph: &'g Graph, name: &str) -> ModuleOverview<'g> {
        let id = graph.id(name).unwrap();
        ModuleOverview::new(&GraphView::new(graph), id)
    }

    #[test]
    fn counts_the_imports_between_children() {
        let graph = graph();
        let overview = overview(&graph, "web/src");
        let names: Vec<_> = overview.children().iter().map(|child| child.name).collect();
        assert_eq!(names, ["web/src/app.ts", "web/src/lib", "web/src/types.ts"]);
        assert_eq!(overview.children()[1].files, 2);
        assert_eq!(
            overview.dependencies(),
            [
                ChildDependency { from: "web/src/app.ts", to: "web/src/lib", imports: 2 },
                ChildDependency { from: "web/src/lib", to: "web/src/types.ts", imports: 2 },
            ]
        );
    }

    #[test]
    fn leaves_out_the_imports_that_the_view_leaves_out() {
        let graph = graph();
        let id = graph.id("web/src").unwrap();
        let view = GraphView::without_kinds(&graph, KindSet::from(ImportKind::Value));
        assert!(ModuleOverview::new(&view, id).dependencies().is_empty());
    }

    #[test]
    fn shows_the_tree_then_the_dependencies() {
        let graph = graph();
        let expected = [
            "web/src/ (4 files)",
            "├─app.ts",
            "├─lib/ (2 files)",
            "│ ├─a.ts",
            "│ └─b.ts",
            "└─types.ts",
            "",
            "dependencies between the children of `web/src`:",
            "  web/src/app.ts -> web/src/lib (2 imports)",
            "  web/src/lib -> web/src/types.ts (2 imports)",
        ];
        assert_eq!(overview(&graph, "web/src").tree(2).to_string(), expected.join("\n") + "\n");

        let root = [". (4 files)", "└─web/ (4 files)", "  └─src/ (4 files)"];
        assert_eq!(overview(&graph, "").tree(2).to_string(), root.join("\n") + "\n");
        assert_eq!(overview(&graph, "web/src/lib").tree(0).to_string(), "web/src/lib/ (2 files)\n");
    }

    #[test]
    fn shows_a_digraph_of_the_children() {
        let graph = graph();
        let expected = [
            "digraph \"web/src/lib\" {",
            "  rankdir=LR;",
            "  node [shape=box];",
            "  \"web/src/lib/a.ts\" [label=\"a.ts\"];",
            "  \"web/src/lib/b.ts\" [label=\"b.ts\"];",
            "}",
        ];
        assert_eq!(overview(&graph, "web/src/lib").dot().to_string(), expected.join("\n") + "\n");

        let expected = [
            "digraph \"web/src\" {",
            "  rankdir=LR;",
            "  node [shape=box];",
            "  \"web/src/app.ts\" [label=\"app.ts\"];",
            "  \"web/src/lib\" [label=\"lib/\"];",
            "  \"web/src/types.ts\" [label=\"types.ts\"];",
            "  \"web/src/app.ts\" -> \"web/src/lib\" [label=\"2\"];",
            "  \"web/src/lib\" -> \"web/src/types.ts\" [label=\"2\"];",
            "}",
        ];
        assert_eq!(overview(&graph, "web/src").dot().to_string(), expected.join("\n") + "\n");
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn serialises_the_children_and_dependencies() {
        let graph = graph();
        let json = serde_json::to_value(overview(&graph, "web/src/lib")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "module": "web/src/lib",
                "children": [
                    { "name": "web/src/lib/a.ts", "kind": "file", "files": 1 },
                    { "name": "web/src/lib/b.ts", "kind": "file", "files": 1 },
                ],
                "dependencies": [],
            })
        );
        let root = serde_json::to_value(overview(&graph, "")).unwrap();
        assert_eq!(root["module"], ".");
        assert_eq!(root["children"][0]["kind"], "directory");
    }

    #[test]
    fn packages_are_not_counted_as_files() {
        let graph = graph_of(&[
            ("cmd/main.go", "internal/db/"),
            ("internal/db/conn.go", "internal/cache/"),
            ("internal/cache/store.go", "node:fmt"),
        ]);
        let overview = overview(&graph, "internal");
        let files: Vec<_> =
            overview.children().iter().map(|child| (child.name, child.files)).collect();
        assert_eq!(files, [("internal/cache", 1), ("internal/db", 1)]);
        assert_eq!(
            overview.dependencies(),
            [ChildDependency { from: "internal/db", to: "internal/cache", imports: 1 }]
        );

        let expected = [
            "internal/ (2 files)",
            "├─cache/ (1 file)",
            "│ └─store.go",
            "└─db/ (1 file)",
            "  └─conn.go",
            "",
            "dependencies between the children of `internal`:",
            "  internal/db -> internal/cache (1 import)",
        ];
        assert_eq!(overview.tree(2).to_string(), expected.join("\n") + "\n");

        let json = serde_json::to_value(&overview).unwrap();
        assert_eq!(
            json["children"][1],
            serde_json::json!({ "name": "internal/db", "kind": "directory", "files": 1 })
        );
    }
}
