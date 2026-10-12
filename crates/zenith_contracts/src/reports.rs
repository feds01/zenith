//! The reports of what checking a contract finds: its violations, and its
//! `ignore_imports` entries that match nothing, in full or a finding each,
//! and its line of the summary.

use zenith_config::{AlertLevel, ContractConfig, ContractKind, Severity};
use zenith_diagnostics::{Report, ReportKind, SourceId, SourceMap, Span};
use zenith_graph::{Graph, ImportDetail, ImportKind};
use zenith_reporting::{Outcome, SummaryEntry};
use zenith_utils::{
    location::ByteRange,
    printing::{plural, thousands},
};

use crate::{Chain, ContractReport, CycleBreaker, Link, Violation};

/// How many excerpts a report of chains or illegal imports shows.
const MAX_EXCERPTS: usize = 3;

/// How many indirect chains a report lists.
const MAX_CHAINS: usize = 5;

/// How many illegal imports a report shows.
const MAX_IMPORTS: usize = 10;

/// How many cycle breakers a report lists.
const MAX_BREAKERS: usize = 5;

impl ContractReport<'_> {
    /// The reports of what checking the contract found: one per violation,
    /// with excerpts of its imports and the contract's guidance, then one per
    /// `ignore_imports` entry that matches nothing, unless they are not to be
    /// reported. The excerpts are read from `sources`, and the entries point
    /// into `config`, the configuration file, if it is there.
    pub fn reports(
        &self,
        graph: &Graph,
        sources: &mut SourceMap,
        config: Option<SourceId>,
    ) -> Vec<Report> {
        ReportBuilder { graph, sources, config }.reports(self)
    }

    /// What checking the contract found, a report each, for showing a line
    /// each: one per chain, illegal import and cycle breaker, pointing at its
    /// first import, and one per `ignore_imports` entry that matches nothing
    /// and breaks the contract.
    pub fn findings(
        &self,
        graph: &Graph,
        sources: &mut SourceMap,
        config: Option<SourceId>,
    ) -> Vec<Report> {
        ReportBuilder { graph, sources, config }.findings(self)
    }

    /// The contract's line of the summary.
    pub fn summary_entry(&self) -> SummaryEntry {
        let outcome = if self.is_kept() {
            Outcome::Kept
        } else {
            Outcome::Broken { findings: self.finding_count(), kind: kind(self.config) }
        };
        let left_out = [(self.ignored, "ignored"), (self.suppressed, "suppressed")];
        let details = left_out
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, how)| format!("{} {how}", thousands(count)))
            .collect();
        SummaryEntry {
            id: self.config.id.clone(),
            kind: self.config.kind.type_name(),
            outcome,
            details,
            duration: self.duration,
        }
    }
}

/// Reports of what a contract violates are errors, or warnings if it is of
/// `warn` severity.
fn kind(config: &ContractConfig) -> ReportKind {
    match config.severity {
        Severity::Error => ReportKind::Error,
        Severity::Warn => ReportKind::Warning,
    }
}

/// Makes the reports of checked contracts, with excerpts of the imports that
/// they are about.
struct ReportBuilder<'a> {
    /// The graph the contracts were checked against.
    graph: &'a Graph,

    /// The project's source files, and the configuration file.
    sources: &'a mut SourceMap,

    /// The configuration file in `sources`, which `ignore_imports` entries
    /// point into, if the configuration was read from a file.
    config: Option<SourceId>,
}

impl ReportBuilder<'_> {
    /// The reports of `report`, as [`ContractReport::reports`] makes them.
    fn reports(&mut self, report: &ContractReport) -> Vec<Report> {
        let config = report.config;
        let kind = kind(config);
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

    /// The findings of `report`, as [`ContractReport::findings`] makes them.
    fn findings(&mut self, report: &ContractReport) -> Vec<Report> {
        let config = report.config;
        let kind = kind(config);
        let encapsulation = matches!(config.kind, ContractKind::Encapsulation(_));
        let graph = self.graph;
        let mut findings = Vec::new();
        for violation in &report.violations {
            match violation {
                Violation::Dependency { importer, imported, chains } => {
                    for chain in chains {
                        let via = match chain.links.len() {
                            1 => String::new(),
                            _ => format!(" (via {})", chain_summary(graph, chain)),
                        };
                        let title = format!("`{importer}` must not import `{imported}`{via}");
                        findings.push(self.finding(kind, title, chain.links.first()));
                    }
                }
                Violation::IllegalImports { module, imports } => {
                    for link in imports {
                        let title = if encapsulation {
                            let imported = graph.name(link.imported);
                            format!(
                                "`{imported}` is internal to `{module}`; import it through an entry point"
                            )
                        } else {
                            let importer = graph.name(link.importer);
                            format!("`{importer}` may not import the protected module `{module}`")
                        };
                        findings.push(self.finding(kind, title, Some(link)));
                    }
                }
                Violation::Cycle { scope, breakers } => {
                    for breaker in breakers {
                        let mut cycle = breaker.cycle.clone();
                        cycle.push(breaker.importer.clone());
                        let title = format!(
                            "dependency cycle {scope}: {}; remove `{}` -> `{}` to break it",
                            cycle.join(" -> "),
                            breaker.importer,
                            breaker.imported
                        );
                        findings.push(self.finding(kind, title, breaker.imports.first()));
                    }
                }
                Violation::UndeclaredModule { container, module } => {
                    let module = graph.name(*module);
                    let title = format!("`{module}` is not declared as a layer of `{container}`");
                    findings.push(Report::new(kind, title));
                }
            }
        }
        for entry in report.unmatched_ignore_errors() {
            let title = format!("the ignored import `{entry}` matches no import");
            let mut finding = Report::new(kind, title);
            if let (Some(source), Some(range)) = (self.config, entry.range()) {
                finding.add_span(Span::new(source, range));
            }
            findings.push(finding);
        }
        findings.into_iter().map(|finding| finding.with_id(&config.id)).collect()
    }

    /// The report of a finding titled `title`, pointing at the first import of
    /// `link` if there is one and its file can be read.
    fn finding(&mut self, kind: ReportKind, title: String, link: Option<&Link>) -> Report {
        let mut report = Report::new(kind, title);
        if let Some(span) = link.and_then(|link| self.span(link)) {
            report.add_span(span);
        }
        report
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
                let encapsulation = matches!(config.kind, ContractKind::Encapsulation(_));
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
        if let Some(span) = self.span(link) {
            report.add_labelled_span(span, label);
        }
    }

    /// The span of the first import of `link`, or `None` if it has none or
    /// its file can't be read.
    fn span(&mut self, link: &Link) -> Option<Span> {
        let import = link.imports.first()?;
        let source = self.sources.file(self.graph.name(link.importer))?;
        Some(Span::new(source, import_span(&self.sources.get(source).contents, import)))
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
    fn lines(&mut self, link: &Link) -> String {
        let source = self.sources.file(self.graph.name(link.importer));
        let lines: Vec<_> = link
            .imports
            .iter()
            .map(|import| match source {
                Some(source) => self.sources.get(source).line(import.range.start()).to_string(),
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
fn import_span(text: &str, import: &ImportDetail) -> ByteRange {
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

/// The nodes of `chain`, as `a.ts -> b.ts -> npm:c`, without the middle of a
/// chain longer than five nodes.
fn chain_summary(graph: &Graph, chain: &Chain) -> String {
    let mut nodes: Vec<_> = chain.links.iter().map(|link| graph.name(link.importer)).collect();
    nodes.extend(chain.links.last().map(|link| graph.name(link.imported)));
    if nodes.len() <= 5 {
        return nodes.join(" -> ");
    }
    let more = nodes.len() - 4;
    format!(
        "{} -> {} -> ({more} more) -> {} -> {}",
        nodes[0],
        nodes[1],
        nodes[nodes.len() - 2],
        nodes[nodes.len() - 1]
    )
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use zenith_graph::{GraphBuilder, ModuleId};

    use super::*;
    use crate::testing::{Project, values};

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
        let text = "import type { A } from './b';\nimport type { B } from './b';\nimport('./b');\n";
        let mut builder = GraphBuilder::new();
        let [a, b] = ["a.ts", "b.ts"].map(|file| builder.add_file(file).unwrap());
        builder.add_import(a, b, detail(text, "import type { A }", "./b", ImportKind::Type));
        let graph = builder.finish();
        let [a, b] = ["a.ts", "b.ts"].map(|file| graph.id(file).unwrap());

        let mut sources = SourceMap::new();
        sources.add("a.ts", text);
        let mut builder = ReportBuilder { graph: &graph, sources: &mut sources, config: None };
        let types = [
            detail(text, "import type { A }", "./b", ImportKind::Type),
            detail(text, "import type { B }", "./b", ImportKind::Type),
        ];
        let link = Link { importer: a, imported: b, imports: types.to_vec() };
        assert_eq!(builder.lines(&link), ":1,2 (type)");

        let mixed = [types[0].clone(), detail(text, "import('./b')", "./b", ImportKind::Dynamic)];
        let link = Link { importer: a, imported: b, imports: mixed.to_vec() };
        assert_eq!(builder.lines(&link), ":1,3");
    }

    #[test]
    fn long_chains_are_summarised_without_their_middle() {
        let mut builder = GraphBuilder::new();
        for n in 0..6 {
            builder.add_file(&format!("{n}.ts")).unwrap();
        }
        let graph = builder.finish();
        let ids: Vec<_> = (0..6).map(|n| graph.id(&format!("{n}.ts")).unwrap()).collect();
        let chain = |nodes: &[ModuleId]| Chain {
            links: nodes
                .windows(2)
                .map(|pair| Link { importer: pair[0], imported: pair[1], imports: Vec::new() })
                .collect(),
            extra_heads: Vec::new(),
            extra_tails: Vec::new(),
        };
        assert_eq!(chain_summary(&graph, &chain(&ids[..3])), "0.ts -> 1.ts -> 2.ts");
        assert_eq!(
            chain_summary(&graph, &chain(&ids[..5])),
            "0.ts -> 1.ts -> 2.ts -> 3.ts -> 4.ts"
        );
        assert_eq!(chain_summary(&graph, &chain(&ids)), "0.ts -> 1.ts -> (2 more) -> 4.ts -> 5.ts");
    }

    #[test]
    fn summary_entries_are_broken_at_the_contract_severity() {
        let contract = |id: &str, from: &str, to: &str, severity: &str| {
            format!(
                "[contracts.{id}]\ntype = \"forbidden\"\nfrom = [\"{from}\"]\nto = [\"{to}\"]\nseverity = \"{severity}\"\n"
            )
        };
        let toml = [
            contract("kept", "b", "a", "error"),
            contract("warned", "a", "b", "warn"),
            contract("broken", "a", "b", "error"),
        ];
        let project = Project::new(&values(&[("a/x.ts", "b/y.ts")]), &[], &toml.concat());
        let outcome = |id: &str| project.check(id).summary_entry().outcome;
        assert_eq!(outcome("kept"), Outcome::Kept);
        assert_eq!(outcome("warned"), Outcome::Broken { findings: 1, kind: ReportKind::Warning });
        assert_eq!(outcome("broken"), Outcome::Broken { findings: 1, kind: ReportKind::Error });
    }

    #[test]
    fn findings_point_at_the_first_import_of_each_chain() {
        let project = Project::new(
            &values(&[("a/x.ts", "b/y.ts"), ("a/z.ts", "c/w.ts"), ("c/w.ts", "b/y.ts")]),
            &[],
            "[contracts.ab]\ntype = \"forbidden\"\nfrom = [\"a\"]\nto = [\"b\"]\n",
        );
        let report = project.check("ab");
        let mut sources = SourceMap::new();
        // The import is at byte 100, the start of the second line.
        sources.add("a/x.ts", format!("{}\nimport x from './x';\n", " ".repeat(99)));
        let findings = report.findings(&project.graph, &mut sources, None);
        let rendered: Vec<_> = findings
            .iter()
            .map(|finding| {
                zenith_diagnostics::Reporter::new(&sources, std::slice::from_ref(finding))
                    .short()
                    .to_string()
            })
            .collect();
        assert_eq!(
            rendered,
            [
                "a/x.ts:2:1: error[ab]: `a` must not import `b`",
                "error[ab]: `a` must not import `b` (via a/z.ts -> c/w.ts -> b/y.ts)",
            ]
        );
    }
}
