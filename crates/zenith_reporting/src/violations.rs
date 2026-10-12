//! The reports of what checking a contract finds: its violations, and its
//! `ignore_imports` entries that match nothing.

use zenith_config::{AlertLevel, ContractConfig, Severity};
use zenith_contracts::{Chain, ContractReport, CycleBreaker, Link, Violation};
use zenith_diagnostics::{Report, ReportKind, SourceId, Span};
use zenith_graph::{Graph, ImportDetail, ImportKind};
use zenith_utils::{location::ByteRange, printing::plural};

use crate::Sources;

/// How many excerpts a report of chains or illegal imports shows.
const MAX_EXCERPTS: usize = 3;

/// How many indirect chains a report lists.
const MAX_CHAINS: usize = 5;

/// How many illegal imports a report shows.
const MAX_IMPORTS: usize = 10;

/// How many cycle breakers a report lists.
const MAX_BREAKERS: usize = 5;

/// Turns what checking contracts finds into reports, with excerpts of the
/// imports that they are about.
pub struct ViolationReporter<'a> {
    /// The graph the contracts were checked against.
    graph: &'a Graph,

    /// The project's source files, and the configuration file.
    sources: &'a mut Sources,

    /// The configuration file in `sources`, which `ignore_imports` entries
    /// point into, if the configuration was read from a file.
    config: Option<SourceId>,
}

impl<'a> ViolationReporter<'a> {
    /// A reporter of the contracts checked against `graph`, reading excerpts
    /// from `sources`. Entries of `ignore_imports` point into the `config`
    /// file, if there is one.
    pub fn new(graph: &'a Graph, sources: &'a mut Sources, config: Option<SourceId>) -> Self {
        Self { graph, sources, config }
    }

    /// The reports of a checked contract: one per violation, then one per
    /// `ignore_imports` entry that matches nothing, unless they are not to
    /// be reported. A contract of `error` severity reports errors, and one of
    /// `warn` severity warnings.
    pub fn reports(&mut self, report: &ContractReport) -> Vec<Report> {
        let config = report.config;
        let kind = match config.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warn => ReportKind::Warning,
        };
        let mut reports: Vec<_> = report
            .violations
            .iter()
            .map(|violation| {
                let mut report = self.violation(kind, config, violation);
                if let Some(guidance) = &config.guidance {
                    report.add_help(guidance.trim());
                }
                report.with_id(&config.id)
            })
            .collect();

        let (kind, help) = match config.unmatched_ignore_imports {
            AlertLevel::Error => {
                (kind, "remove it, or set `unmatched_ignore_imports` to `warn` or `none`")
            }
            AlertLevel::Warn => {
                (ReportKind::Warning, "remove it, or set `unmatched_ignore_imports` to `none`")
            }
            AlertLevel::None => return reports,
        };
        for entry in &report.unmatched_ignores {
            let title = format!("the ignored import `{entry}` matches no import");
            let mut report = Report::new(kind, title).with_id(&config.id);
            if let (Some(source), Some(range)) = (self.config, entry.range()) {
                report.add_span(Span::new(source, range));
            }
            report.add_help(help);
            reports.push(report);
        }
        reports
    }

    /// The report of `violation`, of the contract `config`.
    fn violation(
        &mut self,
        kind: ReportKind,
        config: &ContractConfig,
        violation: &Violation,
    ) -> Report {
        match violation {
            Violation::Dependency { importer, imported, chains } => {
                let title = format!("`{importer}` must not import `{imported}`");
                self.chains(Report::new(kind, title), chains)
            }
            Violation::IllegalImports { module, imports } => {
                let encapsulation = config.kind.type_name() == "encapsulation";
                let title = if encapsulation {
                    format!("`{module}` may only be imported through its entry points")
                } else {
                    format!("`{module}` may only be imported by its allowed importers")
                };
                let mut report = Report::new(kind, title);
                for link in imports.iter().take(MAX_IMPORTS) {
                    let imported = self.graph.name(link.imported);
                    let label = if encapsulation {
                        format!("imports `{imported}`, which is internal")
                    } else {
                        format!("imports `{imported}`")
                    };
                    self.excerpt(&mut report, link, label);
                }
                if imports.len() > MAX_IMPORTS {
                    let more = plural(imports.len() - MAX_IMPORTS, "more import");
                    report.add_note(format!("and {more}"));
                }
                report
            }
            Violation::Cycle { scope, breakers } => {
                self.cycle(Report::new(kind, format!("dependency cycle {scope}")), breakers)
            }
            Violation::UndeclaredModule { container, module } => {
                let module = self.graph.name(*module);
                let title = format!("`{module}` is not declared as a layer of `{container}`");
                let mut report = Report::new(kind, title);
                report.add_help(
                    "the contract is exhaustive: declare it in `layers`, or add it to `exhaustive_ignores`",
                );
                report
            }
        }
    }

    /// Add `chains` to `report`: excerpts of the first link of the first
    /// chains, and the indirect chains link by link.
    fn chains(&mut self, mut report: Report, chains: &[Chain]) -> Report {
        for chain in chains.iter().take(MAX_EXCERPTS) {
            let link = &chain.links[0];
            let label = match chain.links.len() {
                1 => format!("imports `{}`", self.graph.name(link.imported)),
                length => format!("starts a chain of {length} imports"),
            };
            self.excerpt(&mut report, link, label);
        }

        let indirect: Vec<_> = chains.iter().filter(|chain| chain.links.len() > 1).collect();
        for (index, chain) in indirect.iter().take(MAX_CHAINS).enumerate() {
            let heading = match indirect.len() {
                1 => "import chain:".to_string(),
                total => format!("import chain {} of {total}:", index + 1),
            };
            report.add_note(format!("{heading}\n{}", self.chain(chain)));
        }
        if indirect.len() > MAX_CHAINS {
            report.add_note(format!("and {}", plural(indirect.len() - MAX_CHAINS, "more chain")));
        }
        let direct = chains.len() - indirect.len();
        if direct > MAX_EXCERPTS {
            report.add_note(format!(
                "{} in total; use `--format compact` to list them all",
                plural(direct, "direct import")
            ));
        }
        report
    }

    /// Add the cycle `breakers` to `report`: an excerpt of the first import
    /// of the first breaker, and every breaker with its number of imports.
    fn cycle(&mut self, mut report: Report, breakers: &[CycleBreaker]) -> Report {
        if let Some(breaker) = breakers.first()
            && let Some(link) = breaker.imports.first()
        {
            let mut cycle = breaker.cycle.clone();
            cycle.push(breaker.importer.clone());
            self.excerpt(&mut report, link, format!("closes the cycle {}", cycle.join(" -> ")));
        }
        let mut note = format!(
            "it could be made acyclic by removing {}:",
            match breakers.len() {
                1 => "1 dependency".to_string(),
                count => format!("{count} dependencies"),
            }
        );
        for breaker in breakers.iter().take(MAX_BREAKERS) {
            let imports = plural(breaker.imports.len(), "import");
            note += &format!("\n  {} -> {} ({imports})", breaker.importer, breaker.imported);
        }
        if breakers.len() > MAX_BREAKERS {
            note += &format!("\n  and {} more", breakers.len() - MAX_BREAKERS);
        }
        report.add_note(note);
        report
    }

    /// Add an excerpt of the first import of `link`, labelled, to `report`,
    /// if its file can be read.
    fn excerpt(&mut self, report: &mut Report, link: &Link, label: String) {
        let Some(import) = link.imports.first() else {
            return;
        };
        let file = self.graph.name(link.importer);
        let Some(source) = self.sources.source(file) else {
            return;
        };
        let range = import_span(&self.sources.map().get(source).contents, import);
        report.add_labelled_span(Span::new(source, range), label);
    }

    /// `chain` as lines of `file:line` hops, with its other heads and tails
    /// after a `&`.
    fn chain(&mut self, chain: &Chain) -> String {
        let mut lines = Vec::new();
        for (index, link) in chain.links.iter().enumerate() {
            let arrow = if index == 0 { "  " } else { "  -> " };
            lines.push(format!("{arrow}{}{}", self.graph.name(link.importer), self.lines(link)));
            if index == 0 {
                for head in &chain.extra_heads {
                    lines.push(format!(
                        "   & {}{}",
                        self.graph.name(head.importer),
                        self.lines(head)
                    ));
                }
            }
        }
        if let Some(last) = chain.links.last() {
            lines.push(format!("  -> {}", self.graph.name(last.imported)));
            for tail in &chain.extra_tails {
                lines.push(format!("   & {}", self.graph.name(tail.imported)));
            }
        }
        lines.join("\n")
    }

    /// Where the imports of `link` are, e.g. `:3` or `:3,7`, followed by their
    /// kind if they all are of one kind other than value imports, e.g. `:12
    /// (dynamic)`.
    pub(crate) fn lines(&mut self, link: &Link) -> String {
        let file = self.graph.name(link.importer);
        let lines: Vec<_> = link
            .imports
            .iter()
            .map(|import| match self.sources.position(file, import.range.start()) {
                Some((line, _)) => line.to_string(),
                None => "?".to_string(),
            })
            .collect();
        let kind = match link.imports.split_first() {
            Some((first, rest))
                if first.kind != ImportKind::Value
                    && rest.iter().all(|import| import.kind == first.kind) =>
            {
                format!(" ({})", first.kind)
            }
            _ => String::new(),
        };
        if lines.is_empty() { kind } else { format!(":{}{kind}", lines.join(",")) }
    }
}

/// The span to point at for `import` in the `text` of its file: its quoted
/// specifier in the statement, or else the statement's first line.
pub(crate) fn import_span(text: &str, import: &ImportDetail) -> ByteRange {
    let start = (import.range.start() as usize).min(text.len());
    let end = (import.range.end() as usize).clamp(start, text.len());
    let statement = text.get(start..end).unwrap_or_default();
    for quote in ['"', '\'', '`'] {
        let quoted = format!("{quote}{}{quote}", import.specifier);
        if let Some(offset) = statement.find(&quoted) {
            let offset = start + offset;
            return ByteRange::new(offset as u32, (offset + quoted.len()) as u32);
        }
    }
    let line_end = statement.find('\n').map_or(end, |offset| start + offset);
    ByteRange::new(start as u32, line_end as u32)
}

#[cfg(test)]
mod tests {
    use std::{fs, ops::Range};

    use tempfile::TempDir;
    use zenith_graph::GraphBuilder;

    use super::*;

    fn detail(text: &str, statement: &str, specifier: &str, kind: ImportKind) -> ImportDetail {
        let start = text.find(statement).unwrap() as u32;
        let range = ByteRange::new(start, start + statement.len() as u32);
        ImportDetail { range, specifier: specifier.to_string(), kind }
    }

    #[test]
    fn imports_point_at_their_quoted_specifier() {
        let text = "import { a } from './a';\nconst b = require(\"./b\");\nimport(`./c`);\n";
        for (statement, specifier, expected) in [
            ("import { a } from './a';", "./a", "'./a'"),
            ("require(\"./b\")", "./b", "\"./b\""),
            ("import(`./c`)", "./c", "`./c`"),
        ] {
            let range = import_span(text, &detail(text, statement, specifier, ImportKind::Value));
            assert_eq!(&text[Range::from(range)], expected);
        }
    }

    #[test]
    fn imports_without_their_specifier_point_at_their_first_line() {
        let text = "import {\n  a,\n} from './a';\n";
        let mut import = detail(text, "import {\n  a,\n} from './a';", "./a", ImportKind::Value);
        import.specifier = "./elsewhere".to_string();
        assert_eq!(&text[Range::from(import_span(text, &import))], "import {");
    }

    #[test]
    fn links_show_their_lines_and_a_shared_kind() {
        let dir = TempDir::new().unwrap();
        let text = "import type { A } from './b';\nimport type { B } from './b';\nimport('./b');\n";
        fs::write(dir.path().join("a.ts"), text).unwrap();
        let mut builder = GraphBuilder::new();
        let [a, b] = ["a.ts", "b.ts"].map(|file| builder.add_file(file).unwrap());
        builder.add_import(a, b, detail(text, "import type { A }", "./b", ImportKind::Type));
        let graph = builder.finish();
        let [a, b] = ["a.ts", "b.ts"].map(|file| graph.id(file).unwrap());

        let mut sources = Sources::new(dir.path());
        let mut reporter = ViolationReporter::new(&graph, &mut sources, None);
        let types = [
            detail(text, "import type { A }", "./b", ImportKind::Type),
            detail(text, "import type { B }", "./b", ImportKind::Type),
        ];
        let link = Link { importer: a, imported: b, imports: types.to_vec() };
        assert_eq!(reporter.lines(&link), ":1,2 (type)");

        let mixed = [types[0].clone(), detail(text, "import('./b')", "./b", ImportKind::Dynamic)];
        let link = Link { importer: a, imported: b, imports: mixed.to_vec() };
        assert_eq!(reporter.lines(&link), ":1,3");
    }
}
