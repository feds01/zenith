//! The `zenith` subcommands.

pub mod baseline;
pub mod check;
pub mod graph;
pub mod init;
pub mod resolve;
pub mod schema;
pub mod version;
pub mod why;

use anyhow::{Result, anyhow, bail};
use zenith_config::Selector;
use zenith_contracts::Selection;
use zenith_graph::{EdgeSet, Graph, GraphView, ImportKind, KindSet};
use zenith_pipeline::Session;

use crate::cli::ImportKindArg;

/// Parse a selector given on the command line. Commands parse their
/// selectors before indexing the project, so that a typo fails fast.
fn parse_selector(raw: &str) -> Result<Selector> {
    Selector::parse(raw).map_err(|error| anyhow!("invalid selector `{raw}`: {error}"))
}

/// Resolve a selector given on the command line. A path or `pkg:` selector
/// that matches nothing is an error, as in the configuration.
fn select(session: &Session, selector: &Selector) -> Result<Selection> {
    let selection = Selection::resolve(selector, &session.graph, &session.workspace);
    if !selection.matched && selector.must_match() {
        bail!("`{selector}` does not match anything");
    }
    Ok(selection)
}

/// The import kinds that `--exclude-kind` leaves out.
fn excluded_kinds(kinds: &[ImportKindArg]) -> KindSet {
    kinds.iter().map(|kind| ImportKind::from(*kind)).collect()
}

/// A view of `graph` without the imports all of whose statements are of the
/// `excluded` kinds.
fn view_without(graph: &Graph, excluded: KindSet) -> GraphView<'_> {
    if excluded.is_empty() {
        return GraphView::new(graph);
    }
    let imports: EdgeSet = graph
        .edges()
        .filter(|(importer, imported)| {
            graph
                .details(*importer, *imported)
                .iter()
                .all(|detail| excluded.contains(detail.kind.into()))
        })
        .collect();
    GraphView::with_excluded(graph, imports)
}
