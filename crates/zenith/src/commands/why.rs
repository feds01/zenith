//! `zenith why`.

use std::fmt;

use anyhow::{Result, bail};
use zenith_contracts::Selection;
use zenith_diagnostics::SourceMap;
use zenith_graph::{Graph, GraphView, ImportKind, KindSet, ModuleId, NodeSet};
use zenith_pipeline::StageKind;
use zenith_utils::{
    highlight::{Modifier, highlight},
    printing::plural,
};

use crate::{
    ExitStatus,
    cli::{GlobalArgs, WhyCommand},
    project,
};

/// Show the shortest import chains from one selector to another.
///
/// Each link of a chain is shown as the importing file and the lines of its
/// imports of the next node, e.g. `web/src/app.ts:3 -> web/src/api.ts:1 ->
/// npm:sharp`. Imports that aren't plain value imports have their kind next to
/// their line.
pub fn why(global: &GlobalArgs, args: WhyCommand) -> Result<ExitStatus> {
    // 1. Open the project, and build its graph.
    let mut session = project::open(global)?;
    project::run_until(&mut session, StageKind::Graph)?;
    project::warn_about_unresolved(&session);
    let graph = &session.graph;

    // 2. Find the files on either end of the chains.
    let from = Selection::try_resolve(&args.from, graph, &session.workspace)?.members();
    let to = Selection::try_resolve(&args.to, graph, &session.workspace)?.members();
    if from.intersects(&to) {
        bail!("`{}` and `{}` overlap, so there is no import between them", args.from, args.to);
    }

    // 3. Find the shortest chains between them, without the excluded kinds.
    let excluded = args.kinds.excluded();
    let view = GraphView::without_kinds(graph, excluded);
    let chains = view.find_shortest_chains(&from, &to, &NodeSet::new());
    if chains.is_empty() {
        println!("`{}` does not import `{}`.", args.from, args.to);
        return Ok(ExitStatus::Success);
    }

    // 4. Show the chains, up to the limit, with the lines of their imports.
    let mut sources = SourceMap::with_root(&session.root);
    let total = chains.len();
    for (number, chain) in chains.iter().take(args.limit).enumerate() {
        println!("{}", highlight(Modifier::Bold, format!("chain {} of {total}:", number + 1)));
        println!("{}", Chain::new(graph, chain, excluded, &mut sources));
    }
    if total > args.limit {
        println!("and {} more; use `--limit` to show them", plural(total - args.limit, "chain"));
    }
    Ok(ExitStatus::Success)
}

/// An import chain as `zenith why` shows it: a line per importing file, with
/// the lines of its imports of the next node, then a line for the last node.
struct Chain<'g> {
    /// The importing files, in order.
    links: Vec<Link<'g>>,

    /// The node at the end of the chain.
    end: &'g str,
}

/// An importing file in a [`Chain`], and its imports of the next node.
struct Link<'g> {
    /// The importing file.
    importer: &'g str,

    /// Its imports of the next node, in source order.
    imports: Vec<ImportLine>,
}

/// Where an import in a [`Link`] is: its line, followed by its kind if it
/// isn't a plain value import, e.g. `3 (type)`.
struct ImportLine {
    /// The 1-based line, or `None` if the file can't be read any more.
    line: Option<u32>,

    /// How the import is made.
    kind: ImportKind,
}

impl<'g> Chain<'g> {
    /// The chain through the nodes of `chain`, with the imports of each link
    /// that aren't of the `excluded` kinds, located in `sources`.
    fn new(
        graph: &'g Graph,
        chain: &[ModuleId],
        excluded: KindSet,
        sources: &mut SourceMap,
    ) -> Self {
        let links = chain
            .windows(2)
            .map(|link| {
                let [importer, imported] = [link[0], link[1]];
                let file = graph.name(importer);
                let source = sources.file(file).map(|id| sources.get(id));
                let imports = graph
                    .details(importer, imported)
                    .iter()
                    .filter(|detail| !excluded.contains(detail.kind.into()))
                    .map(|detail| ImportLine {
                        line: source.map(|source| source.line(detail.range.start())),
                        kind: detail.kind,
                    })
                    .collect();
                Link { importer: file, imports }
            })
            .collect();
        Self { links, end: graph.name(chain[chain.len() - 1]) }
    }
}

impl fmt::Display for Chain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (position, Link { importer, imports }) in self.links.iter().enumerate() {
            let arrow = if position == 0 { "" } else { "-> " };
            write!(f, "  {arrow}{importer}:")?;
            for (index, import) in imports.iter().enumerate() {
                let separator = if index == 0 { "" } else { ", " };
                write!(f, "{separator}{import}")?;
            }
            writeln!(f)?;
        }
        writeln!(f, "  -> {}", self.end)
    }
}

impl fmt::Display for ImportLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{line}")?,
            None => f.write_str("?")?,
        }
        match self.kind {
            ImportKind::Value => Ok(()),
            kind => write!(f, " ({kind})"),
        }
    }
}
