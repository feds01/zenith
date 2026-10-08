//! `zenith why`: the shortest import chains from one selector to another.

use anyhow::{Result, bail};
use zenith_contracts::Selector;
use zenith_graph::{EdgeSet, GraphView, KindSet, NodeSet, find_shortest_chains};
use zenith_utils::highlight::{Colour, Modifier, highlight};

use crate::{
    ExitStatus,
    cli::{GlobalArgs, WhyCommand},
    project::Project,
};

pub fn why(global: &GlobalArgs, args: WhyCommand) -> Result<ExitStatus> {
    let project = Project::locate(global, false)?;
    let index = project.index()?;
    let graph = &index.graph;

    let members = |raw: &str| -> Result<NodeSet> {
        let selector = Selector::parse(raw).map_err(anyhow::Error::msg)?;
        let units = selector.resolve(&index);
        if units.is_empty() {
            bail!("`{raw}` does not match anything");
        }
        let mut members = NodeSet::new();
        for unit in units {
            members.union_with(&unit.members);
        }
        Ok(members)
    };
    let from = members(&args.from)?;
    let to = members(&args.to)?;
    if from.intersects(&to) {
        bail!("`{}` and `{}` overlap", args.from, args.to);
    }

    let kinds: KindSet = args.exclude_kinds.iter().map(|kind| (*kind).into()).collect();
    let excluded: EdgeSet = graph
        .edges()
        .filter(|(a, b)| graph.details(*a, *b).iter().all(|d| kinds.contains(d.kind)))
        .collect();
    let view = GraphView::with_excluded(graph, excluded);

    let chains = find_shortest_chains(&view, &from, &to, &NodeSet::new());
    if chains.is_empty() {
        println!("`{}` does not import `{}`.", args.from, args.to);
        return Ok(ExitStatus::Success);
    }

    let total = chains.len();
    for (number, chain) in chains.iter().take(args.limit).enumerate() {
        println!("{}", highlight(Modifier::Bold, format!("chain {} of {total}:", number + 1)));
        for (index, pair) in chain.windows(2).enumerate() {
            let details = graph.details(pair[0], pair[1]);
            let lines: Vec<String> = details
                .iter()
                .filter(|d| !kinds.contains(d.kind))
                .map(|d| {
                    if d.kind == zenith_graph::ImportKind::Value {
                        d.line.to_string()
                    } else {
                        format!("{} ({})", d.line, d.kind)
                    }
                })
                .collect();
            let arrow = if index == 0 { "  " } else { "  -> " };
            println!("{arrow}{}:{}", graph.name(pair[0]), highlight(Colour::Cyan, lines.join(",")));
        }
        println!("  -> {}\n", graph.name(*chain.last().unwrap()));
    }
    if total > args.limit {
        println!("and {} more chains (use `--limit` to show them)", total - args.limit);
    }
    Ok(ExitStatus::Success)
}
