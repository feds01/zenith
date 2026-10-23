//! Overviews of the module tree: a node, its children, and the dependencies
//! between them, for showing the shape of part of a project as a tree, a
//! Graphviz digraph, a Mermaid flowchart, or JSON.

use std::fmt;

use serde::Serialize;
use zenith_diagnostics::message;
use zenith_utils::tree_writing::{TreeNode, TreeWriter};

use crate::{Graph, GraphView, ModuleId, NodeKind, NodeSet, UnitGraph};

/// A node of the module tree, its children, and the dependencies between
/// the children in a [`GraphView`].
///
/// It is shown with [`Self::tree`], [`Self::dot`] or [`Self::mermaid`], and
/// serialises as `{"module", "children": [{"name", "kind", "files"}],
/// "dependencies": [{"from", "to", "imports"}]}`.
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
        let Self { from, to, imports } = self;
        f.write_str(&message!(graph_overview_dependency, from, to, imports))
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
            .map(|child| {
                let node = graph.node(*child);
                let (name, kind, files) = (node.name(), node.kind(), graph.file_count(*child));
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

    /// A Mermaid flowchart of the children of each overview and the
    /// dependencies between them, with a subgraph per overview when there are
    /// several.
    pub fn mermaid<'a>(overviews: &'a [Self]) -> OverviewMermaid<'a, 'g> {
        OverviewMermaid { overviews }
    }

    /// Writes the children as Mermaid nodes whose ids are `prefix` and their
    /// position, then the dependencies between them, each line indented by
    /// `indent`.
    fn write_mermaid(&self, f: &mut fmt::Formatter<'_>, indent: &str, prefix: &str) -> fmt::Result {
        let Self { graph, children, dependencies, .. } = self;
        for (index, child) in children.iter().enumerate() {
            writeln!(f, "{indent}{prefix}{index}[{}]", mermaid_label(&child.label(graph)))?;
        }
        let position = |name: &str| children.iter().position(|child| child.name == name);
        for ChildDependency { from, to, imports } in dependencies {
            let (Some(from), Some(to)) = (position(from), position(to)) else { continue };
            writeln!(f, "{indent}{prefix}{from} -->|{imports}| {prefix}{to}")?;
        }
        Ok(())
    }
}

impl OverviewChild<'_> {
    /// The base name of the child, with a trailing `/` for a directory.
    fn label(&self, graph: &Graph) -> String {
        let base_name = graph.node(self.id).base_name();
        match self.kind {
            NodeKind::Directory => format!("{base_name}/"),
            _ => base_name.to_string(),
        }
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
            writeln!(f, "\n{}", message!(graph_overview_dependencies, module))?;
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
            writeln!(f, "  {} [label={}];", quote(child.name), quote(&child.label(graph)))?;
        }
        for ChildDependency { from, to, imports } in dependencies {
            writeln!(f, "  {} -> {} [label=\"{imports}\"];", quote(from), quote(to))?;
        }
        writeln!(f, "}}")
    }
}

/// Shows [`ModuleOverview`]s as one Mermaid flowchart, through
/// [`fmt::Display`]. A single overview is titled with its node; several are
/// each a subgraph titled with theirs. Each child is a node whose id is its
/// position, `n0`, `n1`, …, or `n0_0`, `n0_1`, … in the first subgraph, so
/// that the id is valid whatever the child's path holds.
pub struct OverviewMermaid<'a, 'g> {
    /// The overviews to show.
    overviews: &'a [ModuleOverview<'g>],
}

impl fmt::Display for OverviewMermaid<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let [overview] = self.overviews {
            writeln!(f, "---\ntitle: {}\n---\nflowchart LR", quote(overview.module))?;
            return overview.write_mermaid(f, "  ", "n");
        }
        writeln!(f, "flowchart LR")?;
        for (index, overview) in self.overviews.iter().enumerate() {
            writeln!(f, "  subgraph m{index}[{}]", mermaid_label(overview.module))?;
            overview.write_mermaid(f, "    ", &format!("n{index}_"))?;
            writeln!(f, "  end")?;
        }
        Ok(())
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
    let name = format!("{shown}{slash}");
    let label = message!(graph_overview_directory, name, files = graph.file_count(id));
    if depth == 0 {
        return TreeNode::leaf(label);
    }
    let children =
        graph.children(id).iter().map(|child| tree_node(graph, *child, depth - 1, false));
    TreeNode::branch(label, children.collect())
}

/// The name of a node, with `.` for the root.
fn display_name(graph: &Graph, id: ModuleId) -> &str {
    if id == ModuleId::ROOT { "." } else { graph.name(id) }
}

/// A Graphviz string, which is also a YAML one.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A Mermaid label: quoted, with the characters that Mermaid reads as markup
/// written as entity codes.
fn mermaid_label(text: &str) -> String {
    let mut label = String::from('"');
    for c in text.chars() {
        match c {
            '#' => label.push_str("#35;"),
            '"' => label.push_str("#quot;"),
            '<' => label.push_str("#lt;"),
            '>' => label.push_str("#gt;"),
            _ => label.push(c),
        }
    }
    label.push('"');
    label
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
    fn shows_a_flowchart_of_the_children() {
        let graph = graph();
        let expected = [
            "---",
            "title: \"web/src\"",
            "---",
            "flowchart LR",
            "  n0[\"app.ts\"]",
            "  n1[\"lib/\"]",
            "  n2[\"types.ts\"]",
            "  n0 -->|2| n1",
            "  n1 -->|2| n2",
        ];
        let overviews = [overview(&graph, "web/src")];
        assert_eq!(ModuleOverview::mermaid(&overviews).to_string(), expected.join("\n") + "\n");

        let graph = graph_of(&[
            ("app/@acme/ui-kit/index.ts", "app/v1.2:beta.ts"),
            ("app/v1.2:beta.ts", "app/#1 <b> \"x\".ts"),
        ]);
        let expected = [
            "---",
            "title: \"app\"",
            "---",
            "flowchart LR",
            "  n0[\"#35;1 #lt;b#gt; #quot;x#quot;.ts\"]",
            "  n1[\"@acme/\"]",
            "  n2[\"v1.2:beta.ts\"]",
            "  n1 -->|1| n2",
            "  n2 -->|1| n0",
        ];
        let overviews = [overview(&graph, "app")];
        assert_eq!(ModuleOverview::mermaid(&overviews).to_string(), expected.join("\n") + "\n");
    }

    #[test]
    fn shows_several_overviews_as_subgraphs_of_one_flowchart() {
        let graph = graph();
        let expected = [
            "flowchart LR",
            "  subgraph m0[\"web/src\"]",
            "    n0_0[\"app.ts\"]",
            "    n0_1[\"lib/\"]",
            "    n0_2[\"types.ts\"]",
            "    n0_0 -->|2| n0_1",
            "    n0_1 -->|2| n0_2",
            "  end",
            "  subgraph m1[\"web/src/lib\"]",
            "    n1_0[\"a.ts\"]",
            "    n1_1[\"b.ts\"]",
            "  end",
        ];
        let overviews = [overview(&graph, "web/src"), overview(&graph, "web/src/lib")];
        assert_eq!(ModuleOverview::mermaid(&overviews).to_string(), expected.join("\n") + "\n");
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
