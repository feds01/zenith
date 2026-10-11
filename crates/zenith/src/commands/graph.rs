//! `zenith graph`.

use anyhow::{Result, bail};
use serde_json::{Value, json};
use zenith_graph::{Graph, GraphView, ModuleId, NodeKind, NodeSet, UnitGraph};
use zenith_pipeline::StageKind;
use zenith_utils::{
    printing::plural,
    tree_writing::{TreeNode, TreeWriter},
};

use super::{excluded_kinds, parse_selector, select, view_without};
use crate::{
    ExitStatus,
    cli::{GlobalArgs, GraphCommand, GraphFormat},
    project,
};

/// Show the module tree under a selector, and the dependencies between its
/// children.
pub fn graph(global: &GlobalArgs, args: GraphCommand) -> Result<ExitStatus> {
    let selector = args.selector.as_deref().map(parse_selector).transpose()?;
    let mut session = project::open(global)?;
    project::run_until(&mut session, StageKind::Graph)?;
    let graph = &session.graph;

    let roots: Vec<ModuleId> = match &selector {
        None => vec![ModuleId::ROOT],
        Some(selector) => {
            let selection = select(&session, selector)?;
            if selection.units.is_empty() {
                bail!("`{selector}` does not match anything");
            }
            selection.units.iter().map(|unit| unit.root).collect()
        }
    };
    let view = view_without(graph, excluded_kinds(&args.exclude_kinds));
    let modules: Vec<_> = roots.into_iter().map(|root| Module::new(&view, root)).collect();

    match args.format {
        GraphFormat::Tree => {
            let trees: Vec<_> =
                modules.iter().map(|module| module.tree(graph, args.depth)).collect();
            print!("{}", trees.join("\n"));
        }
        GraphFormat::Dot => {
            for module in &modules {
                print!("{}", module.dot(graph));
            }
        }
        GraphFormat::Json => {
            let modules = modules.iter().map(|module| module.json(graph)).collect();
            println!("{}", serde_json::to_string_pretty(&Value::Array(modules))?);
        }
    }
    Ok(ExitStatus::Success)
}

/// A node that `zenith graph` shows, with the dependencies between its
/// children.
struct Module {
    /// The node.
    id: ModuleId,

    /// The dependencies between the children: the importing child, the
    /// imported child, and the number of imports between their members.
    dependencies: Vec<(ModuleId, ModuleId, u32)>,
}

impl Module {
    /// The node `id`, with the dependencies between its children in `view`.
    fn new(view: &GraphView, id: ModuleId) -> Self {
        let graph = view.graph();
        let children = graph.children(id);
        let units: Vec<NodeSet> = children.iter().map(|child| graph.members(*child)).collect();
        let dependencies = UnitGraph::build(view, &units)
            .dependencies()
            .iter()
            .map(|dependency| {
                let importer = children[dependency.importer.index()];
                (importer, children[dependency.imported.index()], dependency.weight)
            })
            .collect();
        Self { id, dependencies }
    }

    /// The module tree, `depth` levels deep, followed by the dependencies.
    fn tree(&self, graph: &Graph, depth: usize) -> String {
        let mut out = TreeWriter::new(&tree_node(graph, self.id, depth, true)).to_string();
        if !self.dependencies.is_empty() {
            out += &format!("\ndependencies between the children of `{}`:\n", name(graph, self.id));
            for (importer, imported, imports) in &self.dependencies {
                out += &format!(
                    "  {} -> {} ({})\n",
                    graph.name(*importer),
                    graph.name(*imported),
                    plural(*imports as usize, "import")
                );
            }
        }
        out
    }

    /// A Graphviz digraph of the children and the dependencies between them.
    fn dot(&self, graph: &Graph) -> String {
        let mut out = format!("digraph {} {{\n", quote(name(graph, self.id)));
        out += "  rankdir=LR;\n  node [shape=box];\n";
        for child in graph.children(self.id) {
            let node = graph.node(*child);
            let label = match node.kind() {
                NodeKind::Directory => format!("{}/", node.base_name()),
                _ => node.base_name().to_string(),
            };
            out += &format!("  {} [label={}];\n", quote(node.name()), quote(&label));
        }
        for (importer, imported, imports) in &self.dependencies {
            out += &format!(
                "  {} -> {} [label=\"{imports}\"];\n",
                quote(graph.name(*importer)),
                quote(graph.name(*imported))
            );
        }
        out + "}\n"
    }

    /// The children and the dependencies between them, as JSON.
    fn json(&self, graph: &Graph) -> Value {
        let children: Vec<_> = graph
            .children(self.id)
            .iter()
            .map(|child| {
                let kind = match graph.node(*child).kind() {
                    NodeKind::Directory => "directory",
                    NodeKind::File => "file",
                    NodeKind::External => "external",
                    NodeKind::Builtin => "builtin",
                };
                let files = graph.members(*child).len();
                json!({ "name": graph.name(*child), "kind": kind, "files": files })
            })
            .collect();
        let dependencies: Vec<_> = self
            .dependencies
            .iter()
            .map(|(importer, imported, imports)| {
                json!({ "from": graph.name(*importer), "to": graph.name(*imported), "imports": imports })
            })
            .collect();
        json!({ "module": name(graph, self.id), "children": children, "dependencies": dependencies })
    }
}

/// The tree under `id`, `depth` levels deep. The top node is labelled with
/// its full name, the others with their base name. Directories show how many
/// files they hold.
fn tree_node(graph: &Graph, id: ModuleId, depth: usize, top: bool) -> TreeNode {
    let node = graph.node(id);
    let shown = if top { name(graph, id) } else { node.base_name() };
    if node.kind() != NodeKind::Directory {
        return TreeNode::leaf(shown);
    }

    let slash = if id == ModuleId::ROOT { "" } else { "/" };
    let label = format!("{shown}{slash} ({})", plural(graph.members(id).len(), "file"));
    if depth == 0 {
        return TreeNode::leaf(label);
    }
    let children =
        graph.children(id).iter().map(|child| tree_node(graph, *child, depth - 1, false));
    TreeNode::branch(label, children.collect())
}

/// The name of a node, with `.` for the root.
fn name(graph: &Graph, id: ModuleId) -> &str {
    if id == ModuleId::ROOT { "." } else { graph.name(id) }
}

/// A Graphviz string.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}
