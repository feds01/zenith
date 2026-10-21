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
/// their line. A chain through a package, a directory that is imported as a
/// whole, shows it on a line of its own, without lines, between the file that
/// imports it and the file in it that imports on, e.g. `cmd/main.go:3 ->
/// internal/db -> internal/db/conn.go:5 -> internal/cache`.
pub fn why(global: &GlobalArgs, args: WhyCommand) -> Result<ExitStatus> {
    // 1. Open the project, and build its graph.
    let mut session = project::open(global)?;
    project::run_until(&mut session, StageKind::Graph)?;
    project::warn_about_unresolved(&session);
    let graph = &session.graph;

    // 2. Find the files on either end of the chains.
    let from = Selection::try_resolve(&args.from, graph, &session.workspace)?;
    let to = Selection::try_resolve(&args.to, graph, &session.workspace)?;
    let (from, to) = (from.members(), to.members());
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
    /// The importing files, and the packages that the chain goes through, in
    /// order.
    links: Vec<Link<'g>>,

    /// The node at the end of the chain.
    end: &'g str,
}

/// An importing file in a [`Chain`], and its imports of the next node. A
/// package that the chain goes through is a link without imports: it imports
/// nothing, but leads to the files in it.
struct Link<'g> {
    /// The importing file, or the package.
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
                if graph.is_package_member(importer, imported) {
                    return Link { importer: file, imports: Vec::new() };
                }
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
            write!(f, "  {arrow}{importer}")?;
            if !imports.is_empty() {
                f.write_str(":")?;
            }
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

#[cfg(test)]
mod tests {
    use zenith_graph::{GraphBuilder, ImportDetail, ImportedNames};
    use zenith_utils::location::ByteRange;

    use super::*;

    /// An import of `kind` that starts at `start`, of the module `specifier`.
    fn detail(start: u32, specifier: &str, kind: ImportKind) -> ImportDetail {
        ImportDetail {
            range: ByteRange::new(start, start + 10),
            specifier: specifier.into(),
            kind,
            names: ImportedNames::All,
        }
    }

    /// `cmd/main.go` imports the package `internal/db`, whose file imports
    /// the package `internal/cache`, whose file imports `node:os`.
    fn graph() -> Graph {
        let mut builder = GraphBuilder::new();
        let main = builder.add_file("cmd/main.go").unwrap();
        let conn = builder.add_file("internal/db/conn.go").unwrap();
        let store = builder.add_file("internal/cache/store.go").unwrap();
        let os = builder.add_builtin("node:os").unwrap();
        let value = |start, specifier| detail(start, specifier, ImportKind::Value);
        builder.add_package_import(main, "internal/db", value(8, "app/internal/db")).unwrap();
        let cache = detail(24, "app/internal/cache", ImportKind::Type);
        builder.add_package_import(conn, "internal/cache", cache).unwrap();
        builder.add_import(store, os, value(0, "os"));
        builder.finish()
    }

    /// The chains from `from` to `to`, as `zenith why` shows them.
    fn shown(graph: &Graph, from: &str, to: &str) -> Vec<String> {
        let id = |name: &str| graph.id(name).unwrap();
        let view = GraphView::new(graph);
        let (from, to) = (graph.members(id(from)), graph.members(id(to)));
        let chains = view.find_shortest_chains(&from, &to, &NodeSet::new());

        // The import of `cmd/main.go` is on line 9, and the one of
        // `internal/db/conn.go` on line 4.
        let mut sources = SourceMap::new();
        sources.add("cmd/main.go", format!("{}import \"app/internal/db\"\n", "\n".repeat(8)));
        sources.add("internal/db/conn.go", "package db\n\nimport (\n\t\"app/internal/cache\"\n)\n");
        let shown = |chain: &Vec<ModuleId>| {
            Chain::new(graph, chain, KindSet::empty(), &mut sources).to_string()
        };
        chains.iter().map(shown).collect()
    }

    #[test]
    fn an_import_of_a_package_is_shown_by_its_directory() {
        let graph = graph();
        assert_eq!(shown(&graph, "cmd", "internal/db"), ["  cmd/main.go:9\n  -> internal/db\n"]);
    }

    #[test]
    fn a_chain_shows_each_package_between_the_imports_that_make_it() {
        let graph = graph();
        assert_eq!(
            shown(&graph, "cmd", "internal/cache"),
            ["  cmd/main.go:9\n  -> internal/db\n  -> internal/db/conn.go:4 (type)\n  \
                 -> internal/cache\n"]
        );
        // From a file, and through all three.
        assert_eq!(
            shown(&graph, "cmd/main.go", "node:os"),
            ["  cmd/main.go:9\n  -> internal/db\n  -> internal/db/conn.go:4 (type)\n  \
                 -> internal/cache\n  -> internal/cache/store.go:?\n  -> node:os\n"]
        );
    }
}
