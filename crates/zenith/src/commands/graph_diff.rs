//! `zenith graph diff`.

use anyhow::{Result, bail};
use zenith_contracts::Selection;
use zenith_diagnostics::message;
use zenith_graph::{DependencyDiff, GraphView};
use zenith_pipeline::StageKind;
use zenith_workspace::{Checkout, Git};

use crate::{
    ExitStatus,
    cli::{GlobalArgs, GraphDiffCommand, GraphDiffFormat},
    project,
};

/// Show the dependencies between the children of a selector that were added
/// and removed from one git ref to another.
pub fn graph_diff(global: &GlobalArgs, args: GraphDiffCommand) -> Result<ExitStatus> {
    // 1. Open the project in the working tree, which finds its root and
    //    configuration, and find where the head left the base: their merge base, as
    //    in a pull request's diff, so that what the base gained since is not the
    //    head's change.
    let working_tree = project::open(global)?;
    let git = Git::new(&working_tree.root);
    let base_commit = git.branch_point(&args.base, args.head.as_deref().unwrap_or("HEAD"))?;

    // 2. Build the graph at the base, and at the head, or of the working tree, and
    //    warn about the head's unresolved imports only, as `zenith graph` does.
    //    Each checkout is removed when it is dropped, however this returns.
    let base_checkout = git.checkout(&base_commit)?;
    let base = project::build_at(global, &working_tree, &base_checkout, &args.base)?;
    let (head_checkout, head) = match &args.head {
        Some(reference) => {
            let checkout = git.checkout(reference)?;
            let head = project::build_at(global, &working_tree, &checkout, reference)?;
            (Some(checkout), head)
        }
        None => {
            let mut head = working_tree;
            project::run_until(&mut head, StageKind::Graph)?;
            (None, head)
        }
    };
    project::warn_about_unresolved(&head);

    // 3. Follow the files that git sees renamed from the base to the head.
    let head_commit = head_checkout.as_ref().map(Checkout::commit);
    let renames = git.renames_between(base_checkout.commit(), head_commit);

    // 4. Find the nodes to compare in each graph: the roots of the selector's
    //    units, or the project root. The selector need only match at one of them,
    //    such as a package that the head adds.
    let selector = args.selector.as_ref();
    let base_roots = Selection::roots(selector, &base.graph, &base.workspace);
    let head_roots = Selection::roots(selector, &head.graph, &head.workspace);
    if let Some(selector) = selector
        && base_roots.is_empty()
        && head_roots.is_empty()
    {
        bail!(message!(cli_matches_nothing, selector));
    }

    // 5. Compare the dependencies between the nodes below them.
    let excluded = args.kinds.excluded();
    let base_view = GraphView::without_kinds(&base.graph, excluded);
    let head_view = GraphView::without_kinds(&head.graph, excluded);
    let diffs = DependencyDiff::between(
        (&base_view, &base_roots),
        (&head_view, &head_roots),
        args.depth.get(),
        |name| renames.follow(name),
    );

    // 6. Show them in the chosen format.
    match args.format {
        GraphDiffFormat::Tree => {
            let shown: Vec<_> = diffs.iter().map(ToString::to_string).collect();
            print!("{}", shown.join("\n"));
        }
        GraphDiffFormat::Json => println!("{}", serde_json::to_string_pretty(&diffs)?),
    }

    // 7. Drop the sessions before the checkouts that their files are in, so that
    //    nothing of theirs is still open when the directories are removed, which
    //    Windows would refuse.
    drop(head);
    drop(base);
    drop(head_checkout);
    drop(base_checkout);
    Ok(ExitStatus::Success)
}
