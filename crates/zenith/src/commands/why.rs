//! `zenith why`.

use anyhow::{Result, bail};
use zenith_graph::{ImportKind, ModuleId, NodeSet};
use zenith_pipeline::StageKind;
use zenith_utils::{
    highlight::{Modifier, highlight},
    printing::plural,
};

use super::{excluded_kinds, parse_selector, select, view_without};
use crate::{
    ExitStatus,
    cli::{GlobalArgs, WhyCommand},
    project,
    sources::SourceFiles,
};

/// Show the shortest import chains from one selector to another.
///
/// Each link of a chain is shown as the importing file and the lines of its
/// imports of the next node, e.g. `web/src/app.ts:3 -> web/src/api.ts:1 ->
/// npm:sharp`. Imports that aren't plain value imports have their kind next to
/// their line.
pub fn why(global: &GlobalArgs, args: WhyCommand) -> Result<ExitStatus> {
    let (from, to) = (parse_selector(&args.from)?, parse_selector(&args.to)?);
    let mut session = project::open(global)?;
    project::run_until(&mut session, StageKind::Graph)?;
    let graph = &session.graph;

    let from = select(&session, &from)?.members();
    let to = select(&session, &to)?.members();
    if from.intersects(&to) {
        bail!("`{}` and `{}` overlap, so there is no import between them", args.from, args.to);
    }

    let excluded = excluded_kinds(&args.exclude_kinds);
    let view = view_without(graph, excluded);
    let chains = view.find_shortest_chains(&from, &to, &NodeSet::new());
    if chains.is_empty() {
        println!("`{}` does not import `{}`.", args.from, args.to);
        return Ok(ExitStatus::Success);
    }

    let mut sources = SourceFiles::new(&session.root);
    let total = chains.len();
    for (number, chain) in chains.iter().take(args.limit).enumerate() {
        println!("{}", highlight(Modifier::Bold, format!("chain {} of {total}:", number + 1)));
        for (position, link) in chain.windows(2).enumerate() {
            let [importer, imported] = [link[0], link[1]];
            let lines: Vec<_> = graph
                .details(importer, imported)
                .iter()
                .filter(|detail| !excluded.contains(detail.kind.into()))
                .map(|detail| {
                    let file = graph.name(importer);
                    let line = sources.position(file, detail.range.start());
                    let line = line.map_or("?".to_string(), |(line, _)| line.to_string());
                    match detail.kind {
                        ImportKind::Value => line,
                        kind => format!("{line} ({kind})"),
                    }
                })
                .collect();
            let arrow = if position == 0 { "  " } else { "  -> " };
            println!("{arrow}{}:{}", graph.name(importer), lines.join(", "));
        }
        let last: ModuleId = chain[chain.len() - 1];
        println!("  -> {}\n", graph.name(last));
    }
    if total > args.limit {
        println!("and {} more; use `--limit` to show them", plural(total - args.limit, "chain"));
    }
    Ok(ExitStatus::Success)
}
