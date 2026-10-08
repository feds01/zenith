//! `zenith graph`: the module tree under a selector, and the dependencies
//! between its children.

use anyhow::{Result, bail};
use serde_json::json;
use zenith_contracts::Selector;
use zenith_graph::{EdgeSet, Graph, GraphView, KindSet, ModuleId, NodeKind, UnitGraph};
use zenith_utils::tree_writing::{TreeNode, TreeWriter};

use crate::{
    ExitStatus,
    cli::{GlobalArgs, GraphCommand, GraphFormat},
    project::Project,
};

pub fn graph(global: &GlobalArgs, args: GraphCommand) -> Result<ExitStatus> {
    let project = Project::locate(global, false)?;
    let index = project.index()?;
    let graph = &index.graph;

    let roots: Vec<ModuleId> = match &args.selector {
        None => vec![ModuleId::ROOT],
        Some(raw) => {
            let selector = Selector::parse(raw).map_err(anyhow::Error::msg)?;
            let roots: Vec<_> =
                selector.resolve(&index).into_iter().filter_map(|unit| unit.root).collect();
            if roots.is_empty() {
                bail!("`{raw}` does not match anything");
            }
            roots
        }
    };

    let kinds: KindSet = args.exclude_kinds.iter().map(|kind| (*kind).into()).collect();
    let excluded: EdgeSet = graph
        .edges()
        .filter(|(a, b)| graph.details(*a, *b).iter().all(|d| kinds.contains(d.kind)))
        .collect();
    let view = GraphView::with_excluded(graph, excluded);

    for root in roots {
        match args.format {
            GraphFormat::Tree => {
                let tree = tree_node(graph, root, args.depth);
                print!("{}", TreeWriter::new(&tree));
                let dependencies = child_dependencies(&view, root);
                if !dependencies.is_empty() {
                    println!(
                        "\ndependencies between the children of `{}`:",
                        display_name(graph, root)
                    );
                    for (from, to, count) in dependencies {
                        println!(
                            "  {from} -> {to} ({count} import{})",
                            if count == 1 { "" } else { "s" }
                        );
                    }
                }
                println!();
            }
            GraphFormat::Dot => {
                println!("digraph \"{}\" {{", display_name(graph, root));
                println!("  rankdir=LR;\n  node [shape=box, fontname=\"Helvetica\"];");
                for child in graph.children(root) {
                    let node = graph.node(*child);
                    let label = if node.kind == NodeKind::Directory {
                        format!("{}/", node.base_name())
                    } else {
                        node.base_name().to_string()
                    };
                    println!("  \"{}\" [label=\"{label}\"];", node.name);
                }
                for (from, to, count) in child_dependencies(&view, root) {
                    println!("  \"{from}\" -> \"{to}\" [label=\"{count}\"];");
                }
                println!("}}");
            }
            GraphFormat::Json => {
                let children: Vec<_> = graph
                    .children(root)
                    .iter()
                    .map(|child| json!({ "name": graph.name(*child), "files": graph.members(*child).len() }))
                    .collect();
                let dependencies: Vec<_> = child_dependencies(&view, root)
                    .into_iter()
                    .map(
                        |(from, to, imports)| json!({ "from": from, "to": to, "imports": imports }),
                    )
                    .collect();
                let value = json!({ "module": graph.name(root), "children": children, "dependencies": dependencies });
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
        }
    }
    Ok(ExitStatus::Success)
}

fn display_name(graph: &Graph, id: ModuleId) -> String {
    if id == ModuleId::ROOT { ".".to_string() } else { graph.name(id).to_string() }
}

fn tree_node(graph: &Graph, id: ModuleId, depth: usize) -> TreeNode {
    let node = graph.node(id);
    let label = match node.kind {
        NodeKind::Directory => {
            let files = graph.members(id).len();
            let name = if id == ModuleId::ROOT {
                ".".to_string()
            } else {
                format!("{}/", node.base_name())
            };
            format!("{name} ({files} file{})", if files == 1 { "" } else { "s" })
        }
        _ => node.base_name().to_string(),
    };
    if depth == 0 || node.children.is_empty() {
        return TreeNode::leaf(label);
    }
    TreeNode::branch(
        label,
        node.children.iter().map(|child| tree_node(graph, *child, depth - 1)).collect(),
    )
}

/// `(from, to, imports)` between the children of `root`.
fn child_dependencies(view: &GraphView, root: ModuleId) -> Vec<(String, String, usize)> {
    let graph = view.graph();
    let children = graph.children(root);
    let units: Vec<_> = children.iter().map(|child| graph.members(*child)).collect();
    let unit_graph = UnitGraph::build(view, &units);
    unit_graph
        .weights
        .iter()
        .map(|((from, to), count)| {
            (graph.name(children[*from]).to_string(), graph.name(children[*to]).to_string(), *count)
        })
        .collect()
}
