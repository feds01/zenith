//! `zenith graph`.

use anyhow::{Result, bail};
use zenith_contracts::Selection;
use zenith_graph::{GraphView, ModuleId, ModuleOverview};
use zenith_pipeline::StageKind;

use crate::{
    ExitStatus,
    cli::{GlobalArgs, GraphCommand, GraphFormat},
    project,
};

/// Show the module tree under a selector, and the dependencies between its
/// children.
pub fn graph(global: &GlobalArgs, args: GraphCommand) -> Result<ExitStatus> {
    // 1. Open the project, and build its graph.
    let mut session = project::open(global)?;
    project::run_until(&mut session, StageKind::Graph)?;
    project::warn_about_unresolved(&session);
    let graph = &session.graph;

    // 2. Find the nodes to show: the roots of the selector's units, or the project
    //    root. A selector without units has nothing to show, even one that may
    //    match nothing, such as an `npm:` package.
    let roots: Vec<ModuleId> = match &args.selector {
        None => vec![ModuleId::ROOT],
        Some(selector) => {
            let selection = Selection::resolve(selector, graph, &session.workspace);
            if selection.units.is_empty() {
                bail!("`{selector}` does not match anything");
            }
            selection.units.iter().map(|unit| unit.root).collect()
        }
    };

    // 3. Gather the children of each node, and the dependencies between them.
    let view = GraphView::without_kinds(graph, args.kinds.excluded());
    let overviews: Vec<_> =
        roots.into_iter().map(|root| ModuleOverview::new(&view, root)).collect();

    // 4. Show them in the chosen format.
    match args.format {
        GraphFormat::Tree => {
            let trees: Vec<_> =
                overviews.iter().map(|overview| overview.tree(args.depth).to_string()).collect();
            print!("{}", trees.join("\n"));
        }
        GraphFormat::Dot => {
            for overview in &overviews {
                print!("{}", overview.dot());
            }
        }
        GraphFormat::Json => println!("{}", serde_json::to_string_pretty(&overviews)?),
    }
    Ok(ExitStatus::Success)
}
