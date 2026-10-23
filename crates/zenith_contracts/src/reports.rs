//! The reports of what checking a contract finds: its violations, and its
//! `ignore_imports` entries that match nothing, in full or a finding each,
//! and its line of the summary.

use zenith_config::{AlertLevel, BaselineConfig, ContractConfig, ContractKind, Severity};
use zenith_diagnostics::{Report, ReportElement, ReportKind, SourceId, SourceMap, Span, message};
use zenith_graph::{Graph, ImportDetail, ImportKind};
use zenith_reporting::{Count, Outcome, Summary, SummaryEntry};
use zenith_utils::location::ByteRange;

use crate::{
    Chain, ContractReport, CycleBreaker, CycleScope, Fix, Link, Violation, baseline::entry_ranges,
};

/// How many excerpts a report of chains or illegal imports shows.
const MAX_EXCERPTS: usize = 3;

/// How many indirect chains a report lists.
const MAX_CHAINS: usize = 5;

/// How many illegal imports a report shows.
const MAX_IMPORTS: usize = 10;

/// How many cycle breakers a report lists.
const MAX_BREAKERS: usize = 5;

/// How many stale baseline entries a report points at.
const MAX_STALE: usize = 10;

impl ContractReport<'_> {
    /// The reports of what checking the contract found: one per violation,
    /// with excerpts of its imports, how to fix it and the contract's
    /// guidance, one per `ignore_imports` entry that matches nothing, unless
    /// they are not to be reported, and one of its stale `baseline` entries.
    /// The excerpts are read from `sources`, and the entries point into
    /// `config`, the configuration file, if it is there, and the baseline
    /// file.
    pub fn reports(
        &self,
        graph: &Graph,
        sources: &mut SourceMap,
        config: Option<SourceId>,
        baseline: &BaselineConfig,
    ) -> Vec<Report> {
        ReportBuilder { graph, sources, config, baseline }.reports(self)
    }

    /// What checking the contract found, a report each, for showing a line
    /// each: one per chain, illegal import, cycle breaker, undeclared module
    /// and unit over a limit, pointing at its first import if it has one,
    /// with how to fix it and the contract's guidance, one per
    /// `ignore_imports` entry that matches nothing, unless they are not to be
    /// reported, and one per stale `baseline` entry.
    pub fn findings(
        &self,
        graph: &Graph,
        sources: &mut SourceMap,
        config: Option<SourceId>,
        baseline: &BaselineConfig,
    ) -> Vec<Report> {
        let findings = self.identified_findings(graph, sources, config, baseline);
        findings.into_iter().map(|finding| finding.report).collect()
    }

    /// The findings of the contract, as [`ContractReport::findings`] makes
    /// them, each with what tells it apart from run to run and the links
    /// after the one it points at, as SARIF shows them.
    pub(crate) fn identified_findings(
        &self,
        graph: &Graph,
        sources: &mut SourceMap,
        config: Option<SourceId>,
        baseline: &BaselineConfig,
    ) -> Vec<Finding<'_>> {
        ReportBuilder { graph, sources, config, baseline }.findings(self)
    }

    /// What checking the contract found, as diagnostics for an editor to
    /// show on the code: the [findings](Self::findings), each also labelling
    /// the first import of its related links, the rest of its chain or the
    /// other imports of its cycle breaker, which an editor shows as related
    /// information. A finding about no one import, such as an undeclared
    /// module or an exceeded limit, points at none.
    pub fn diagnostics(
        &self,
        graph: &Graph,
        sources: &mut SourceMap,
        config: Option<SourceId>,
        baseline: &BaselineConfig,
    ) -> Vec<Report> {
        let mut builder = ReportBuilder { graph, sources, config, baseline };
        let findings = builder.findings(self);
        findings.into_iter().map(|finding| builder.with_related(finding)).collect()
    }

    /// The summary of the checked contracts `reports`, with the `baseline`
    /// configured: a line for each, and how many stale baseline entries they
    /// have. The counts of files and imports, and the timings, are left out.
    pub fn summary(reports: &[Self], baseline: &BaselineConfig) -> Summary {
        let entries = reports.iter().map(|report| report.summary_entry(baseline)).collect();
        let count = reports.iter().map(|report| report.stale_entries.len()).sum();
        let stale = Count { text: message!(contracts_stale_entries, count), count };
        Summary { entries, counts: vec![stale], ..Summary::default() }
    }

    /// The contract's line of the summary, with the `baseline` configured.
    pub fn summary_entry(&self, baseline: &BaselineConfig) -> SummaryEntry {
        let outcome = if !self.is_kept() {
            Outcome::Broken { findings: self.finding_count(), kind: kind(self.config) }
        } else if self.stale_entries.is_empty() {
            Outcome::Kept
        } else {
            let status = message!(contracts_kept_with_stale_baseline);
            Outcome::KeptWith { status, kind: baseline.stale_kind(self.config.severity) }
        };
        let left_out = [
            (self.baselined, message!(contracts_baselined, count = self.baselined)),
            (self.ignored, message!(contracts_ignored, count = self.ignored)),
            (self.suppressed, message!(contracts_suppressed, count = self.suppressed)),
        ];
        let details = left_out
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(_, detail)| detail)
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
pub(crate) fn kind(config: &ContractConfig) -> ReportKind {
    match config.severity {
        Severity::Error => ReportKind::Error,
        Severity::Warn => ReportKind::Warning,
    }
}

/// A finding of a contract: its report, a line's worth, with its title on
/// its own, what tells it apart from the other findings of the contract from
/// run to run, the links that it is also about, and how to fix it.
pub(crate) struct Finding<'r> {
    /// The report, pointing at the first import of the finding, if it has
    /// one, and titled with the finding and the hint of its fix.
    pub report: Report,

    /// What the finding is, without the hint of its fix.
    pub title: String,

    /// What tells the finding apart from run to run, whatever lines its
    /// imports move to: the units, files and entries that it is about.
    pub identity: String,

    /// The links after the one that the report points at: the rest of a
    /// chain, or the other imports of a cycle breaker, up to [`MAX_IMPORTS`].
    pub related: &'r [Link],

    /// How to fix it, if it is a finding of a violation.
    pub fix: Option<Fix<'r>>,
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

    /// How the baseline is configured: where its file is, and whether stale
    /// entries are errors.
    baseline: &'a BaselineConfig,
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
                report.add_help(violation.hint(&config.kind));
                if let Some(guidance) = &config.guidance {
                    report.add_help(guidance.trim());
                }
                report.with_id(&config.id)
            })
            .collect();

        reports.extend(self.unmatched_ignores(kind, report));
        reports.extend(self.stale(report));
        reports
    }

    /// The reports of the `ignore_imports` entries of `report` that match
    /// nothing, of `kind` if they break it, unless they are not to be
    /// reported.
    fn unmatched_ignores(&self, kind: ReportKind, report: &ContractReport) -> Vec<Report> {
        let config = report.config;
        let (kind, help) = match config.unmatched_ignore_imports {
            AlertLevel::Error => (kind, message!(contracts_unmatched_ignore.help_error)),
            AlertLevel::Warn => {
                (ReportKind::Warning, message!(contracts_unmatched_ignore.help_warn))
            }
            AlertLevel::None => return Vec::new(),
        };
        let mut reports = Vec::new();
        for entry in &report.unmatched_ignores {
            let title = message!(contracts_unmatched_ignore, entry);
            let mut report = Report::new(kind, title).with_id(&config.id);
            if let (Some(source), Some(range)) = (self.config, entry.range()) {
                report.add_span(Span::new(source, range));
            }
            report.add_help(&help);
            reports.push(report);
        }
        reports
    }

    /// The report of the stale baseline entries of `report`, pointing at them
    /// in the baseline file, or `None` if it has none.
    fn stale(&mut self, report: &ContractReport) -> Option<Report> {
        let entries = &report.stale_entries;
        if entries.is_empty() {
            return None;
        }
        let config = report.config;
        let title = message!(contracts_stale_entries, count = entries.len());
        let kind = self.baseline.stale_kind(config.severity);
        let mut stale = Report::new(kind, title).with_id(&config.id);
        let mut unlocated = Vec::new();
        let shown = &entries[..entries.len().min(MAX_STALE)];
        for (entry, span) in shown.iter().zip(self.entry_spans(&config.id, shown)) {
            match span {
                Some(span) => {
                    stale.add_labelled_span(span, message!(contracts_stale_entries.label));
                }
                None => unlocated.push(format!("  {entry}")),
            }
        }
        if !unlocated.is_empty() {
            let heading = message!(contracts_stale_entries.unlocated);
            stale.add_note(format!("{heading}\n{}", unlocated.join("\n")));
        }
        if entries.len() > MAX_STALE {
            stale.add_note(message!(contracts_more_entries, count = entries.len() - MAX_STALE));
        }
        stale.add_help(message!(contracts_stale_entries.help, count = entries.len()));
        Some(stale)
    }

    /// Where each of the `entries` of the contract `id` is in the baseline
    /// file, or `None` if the file can't be read or the entry found in it.
    fn entry_spans(&mut self, id: &str, entries: &[String]) -> Vec<Option<Span>> {
        let Some(source) = self.sources.file(&self.baseline.path) else {
            return vec![None; entries.len()];
        };
        let ranges = entry_ranges(&self.sources.get(source).contents, id, entries);
        ranges.into_iter().map(|range| range.map(|range| Span::new(source, range))).collect()
    }

    /// The findings of `report`, as [`ContractReport::identified_findings`]
    /// makes them.
    fn findings<'r>(&mut self, report: &'r ContractReport) -> Vec<Finding<'r>> {
        let config = report.config;
        let kind = kind(config);
        let encapsulation = matches!(config.kind, ContractKind::Encapsulation(_));
        let graph = self.graph;
        let mut findings = Vec::new();
        for violation in &report.violations {
            // Each finding's title, identity and related links, in the order of its fix.
            let parts: Vec<(String, String, &[Link])> = match violation {
                Violation::Dependency { importer, imported, chains } => chains
                    .iter()
                    .map(|chain| {
                        let title = match chain.links.len() {
                            1 => message!(contracts_must_not_import, importer, imported),
                            _ => {
                                let chain = chain_summary(graph, chain);
                                message!(contracts_must_not_import_via, importer, imported, chain)
                            }
                        };
                        let nodes: Vec<_> =
                            chain.nodes().into_iter().map(|node| graph.name(node)).collect();
                        let identity =
                            identity(&["dependency", importer, imported, &nodes.join(" -> ")]);
                        (title, identity, related(&chain.links))
                    })
                    .collect(),
                Violation::IllegalImports { module, imports } => imports
                    .iter()
                    .map(|link| {
                        let (importer, imported) =
                            (graph.name(link.importer), graph.name(link.imported));
                        let title = if encapsulation {
                            message!(contracts_internal_import, imported, module)
                        } else {
                            message!(contracts_protected_import, importer, module)
                        };
                        let identity = identity(&["illegal-import", module, importer, imported]);
                        (title, identity, &[][..])
                    })
                    .collect(),
                Violation::Cycle { scope, breakers } => {
                    let units = match scope {
                        CycleScope::Modules(selectors) => ["modules", selectors.as_str()],
                        CycleScope::Children(unit) => ["children", unit.as_str()],
                    };
                    breakers
                        .iter()
                        .map(|breaker| {
                            let mut cycle = breaker.cycle.clone();
                            cycle.push(breaker.importer.clone());
                            let title = message!(
                                contracts_cycle_finding,
                                scope,
                                cycle = cycle.join(" -> ")
                            );
                            let (importer, imported) = (&breaker.importer, &breaker.imported);
                            let identity =
                                identity(&["cycle", units[0], units[1], importer, imported]);
                            (title, identity, related(&breaker.imports))
                        })
                        .collect()
                }
                Violation::UndeclaredModule { container, module } => {
                    let module = graph.name(*module);
                    let title = message!(contracts_undeclared_module, module, container);
                    vec![(title, identity(&["undeclared-module", container, module]), &[][..])]
                }
                // The finding is at no import, so each import that makes up the count is
                // related to it.
                Violation::LimitExceeded { module, limit, count, max, imports } => {
                    let title = limit.title(module, *count, *max);
                    let identity = identity(&["limit-exceeded", module, limit.name()]);
                    vec![(title, identity, &imports[..imports.len().min(MAX_IMPORTS)])]
                }
            };
            for ((title, identity, related), fix) in
                parts.into_iter().zip(violation.fixes(&config.kind))
            {
                let report = self.finding(kind, &title, &fix);
                findings.push(Finding { report, title, identity, related, fix: Some(fix) });
            }
        }
        if let Some(guidance) = &config.guidance {
            for finding in &mut findings {
                finding.report.add_help(guidance.trim());
            }
        }

        let unmatched = match config.unmatched_ignore_imports {
            AlertLevel::Error => Some(kind),
            AlertLevel::Warn => Some(ReportKind::Warning),
            AlertLevel::None => None,
        };
        if let Some(unmatched) = unmatched {
            for entry in &report.unmatched_ignores {
                let title = message!(contracts_unmatched_ignore, entry);
                let mut finding = Report::new(unmatched, title);
                if let (Some(source), Some(range)) = (self.config, entry.range()) {
                    finding.add_span(Span::new(source, range));
                }
                let identity = identity(&["unmatched-ignore", entry.as_str()]);
                let title = finding.title.clone();
                findings.push(Finding {
                    report: finding,
                    title,
                    identity,
                    related: &[],
                    fix: None,
                });
            }
        }
        let stale = self.entry_spans(&config.id, &report.stale_entries);
        for (entry, span) in report.stale_entries.iter().zip(stale) {
            let title = message!(contracts_stale_entry, entry);
            let mut finding = Report::new(self.baseline.stale_kind(config.severity), title);
            if let Some(span) = span {
                finding.add_span(span);
            }
            finding.add_help(message!(contracts_stale_entries.help, count = 1));
            let identity = identity(&["stale-entry", entry]);
            let title = finding.title.clone();
            findings.push(Finding { report: finding, title, identity, related: &[], fix: None });
        }
        for finding in &mut findings {
            finding.report.id = Some(config.id.clone());
        }
        findings
    }

    /// The report of `finding`, which, if it points at an import, also labels
    /// the first import of each of its related links with what it imports.
    fn with_related(&mut self, finding: Finding) -> Report {
        let mut report = finding.report;
        let located =
            report.contents.iter().any(|element| matches!(element, ReportElement::CodeBlock(_)));
        if located {
            for link in finding.related {
                let label = message!(contracts_imports, imported = self.graph.name(link.imported));
                self.excerpt(&mut report, link, label);
            }
        }
        report
    }

    /// The report of a finding titled `title`, followed by the hint of its
    /// `fix`, pointing at the first import of the fix's link if there is one
    /// and its file can be read.
    fn finding(&mut self, kind: ReportKind, title: &str, fix: &Fix) -> Report {
        let mut report =
            Report::new(kind, message!(contracts_finding, finding = title, hint = fix.hint));
        if let Some(span) = fix.link.and_then(|link| self.span(link)) {
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
                let title = message!(contracts_must_not_import, importer, imported);
                self.chains(Report::new(kind, title), chains)
            }
            Violation::IllegalImports { module, imports } => {
                let encapsulation = matches!(config.kind, ContractKind::Encapsulation(_));
                let title = if encapsulation {
                    message!(contracts_internal_module, module)
                } else {
                    message!(contracts_protected_module, module)
                };
                self.imports(Report::new(kind, title), imports, |imported| {
                    if encapsulation {
                        message!(contracts_internal_module.label, imported)
                    } else {
                        message!(contracts_imports, imported)
                    }
                })
            }
            Violation::Cycle { scope, breakers } => {
                self.cycle(Report::new(kind, message!(contracts_cycle, scope)), breakers)
            }
            Violation::UndeclaredModule { container, module } => {
                let module = self.graph.name(*module);
                let title = message!(contracts_undeclared_module, module, container);
                Report::new(kind, title)
            }
            Violation::LimitExceeded { module, limit, count, max, imports } => {
                let report = Report::new(kind, limit.title(module, *count, *max));
                self.imports(report, imports, |imported| message!(contracts_imports, imported))
            }
        }
    }

    /// Add `imports` to `report`: excerpts of the first ones, each labelled
    /// by `label` from the name of the node it imports, and how many more
    /// there are.
    fn imports(
        &mut self,
        mut report: Report,
        imports: &[Link],
        label: impl Fn(&str) -> String,
    ) -> Report {
        for link in imports.iter().take(MAX_IMPORTS) {
            let label = label(self.graph.name(link.imported));
            self.excerpt(&mut report, link, label);
        }
        if imports.len() > MAX_IMPORTS {
            report.add_note(message!(contracts_more_imports, count = imports.len() - MAX_IMPORTS));
        }
        report
    }

    /// Add `chains` to `report`: excerpts of the first link of the first
    /// chains, and the indirect chains link by link.
    fn chains(&mut self, mut report: Report, chains: &[Chain]) -> Report {
        for chain in chains.iter().take(MAX_EXCERPTS) {
            let link = &chain.links[0];
            let label = match chain.links.len() {
                1 => message!(contracts_imports, imported = self.graph.name(link.imported)),
                length => message!(contracts_chain_start, length),
            };
            self.excerpt(&mut report, link, label);
        }

        let indirect: Vec<_> = chains.iter().filter(|chain| chain.links.len() > 1).collect();
        for (index, chain) in indirect.iter().take(MAX_CHAINS).enumerate() {
            let heading =
                message!(contracts_chain_heading, index = index + 1, total = indirect.len());
            report.add_note(format!("{heading}\n{}", self.chain(chain)));
        }
        if indirect.len() > MAX_CHAINS {
            report.add_note(message!(contracts_more_chains, count = indirect.len() - MAX_CHAINS));
        }
        let direct = chains.len() - indirect.len();
        if direct > MAX_EXCERPTS {
            report.add_note(message!(contracts_direct_imports, count = direct));
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
            let label = message!(contracts_cycle.label, cycle = cycle.join(" -> "));
            self.excerpt(&mut report, link, label);
        }
        let mut note = message!(contracts_cycle_breakers, count = breakers.len());
        for breaker in breakers.iter().take(MAX_BREAKERS) {
            let line = message!(
                contracts_cycle_breaker,
                importer = breaker.importer,
                imported = breaker.imported,
                count = breaker.imports.len()
            );
            note += &format!("\n  {line}");
        }
        if breakers.len() > MAX_BREAKERS {
            let more = message!(contracts_more_breakers, count = breakers.len() - MAX_BREAKERS);
            note += &format!("\n  {more}");
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
        link.span(self.graph, self.sources)
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
            // A package that the chain goes through is the line between the file that
            // imports it and the file in it that imports on.
            if let Some(next) = chain.links.get(index + 1)
                && next.importer != link.imported
            {
                lines.push(format!("  -> {}", self.graph.name(link.imported)));
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

impl Link {
    /// The span of the first import of the link, of `graph`, in `sources`, or
    /// `None` if it has none or its file can't be read.
    pub(crate) fn span(&self, graph: &Graph, sources: &mut SourceMap) -> Option<Span> {
        let import = self.imports.first()?;
        let source = sources.file(graph.name(self.importer))?;
        Some(Span::new(source, import_span(&sources.get(source).contents, import)))
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

/// The identity of a finding made of `parts`, which have no NUL.
pub(crate) fn identity(parts: &[&str]) -> String {
    parts.join("\0")
}

/// The links after the first of `links`, up to [`MAX_IMPORTS`].
fn related(links: &[Link]) -> &[Link] {
    let rest = links.get(1..).unwrap_or_default();
    &rest[..rest.len().min(MAX_IMPORTS)]
}

/// The nodes of `chain`, as `a.ts -> b.ts -> npm:c`, without the middle of a
/// chain longer than five nodes.
fn chain_summary(graph: &Graph, chain: &Chain) -> String {
    let nodes: Vec<_> = chain.nodes().into_iter().map(|node| graph.name(node)).collect();
    if nodes.len() <= 5 {
        return nodes.join(" -> ");
    }
    let more = message!(contracts_chain_middle, count = nodes.len() - 4);
    format!(
        "{} -> {} -> {more} -> {} -> {}",
        nodes[0],
        nodes[1],
        nodes[nodes.len() - 2],
        nodes[nodes.len() - 1]
    )
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use zenith_diagnostics::ReportCodeBlock;
    use zenith_graph::{GraphBuilder, ImportedNames, ModuleId};

    use super::*;
    use crate::testing::{Project, values};

    fn detail(text: &str, statement: &str, specifier: &str, kind: ImportKind) -> ImportDetail {
        let start = text.find(statement).unwrap() as u32;
        let range = ByteRange::new(start, start + statement.len() as u32);
        ImportDetail { range, specifier: specifier.into(), kind, names: ImportedNames::All }
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
        import.specifier = "./elsewhere".into();
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
        let baseline = BaselineConfig::default();
        let mut builder = ReportBuilder {
            graph: &graph,
            sources: &mut sources,
            config: None,
            baseline: &baseline,
        };
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
    fn counts_choose_their_plural_and_group_their_digits() {
        assert_eq!(message!(contracts_chain_heading, index = 1, total = 1), "import chain:");
        assert_eq!(message!(contracts_chain_heading, index = 2, total = 3), "import chain 2 of 3:");
        assert_eq!(message!(contracts_more_chains, count = 1), "and 1 more chain");
        assert_eq!(message!(contracts_more_imports, count = 1234), "and 1,234 more imports");
        assert_eq!(message!(contracts_more_entries, count = 2), "and 2 more entries");
        assert_eq!(
            message!(contracts_cycle_breakers, count = 2),
            "it could be made acyclic by removing 2 dependencies:"
        );
        assert_eq!(
            message!(contracts_stale_entries.help, count = 2),
            "run `zenith baseline --prune` to remove them"
        );
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
        let baseline = BaselineConfig::default();
        let outcome = |id: &str| project.check(id).summary_entry(&baseline).outcome;
        assert_eq!(outcome("kept"), Outcome::Kept);
        assert_eq!(outcome("warned"), Outcome::Broken { findings: 1, kind: ReportKind::Warning });
        assert_eq!(outcome("broken"), Outcome::Broken { findings: 1, kind: ReportKind::Error });
    }

    #[test]
    fn stale_baseline_entries_fail_the_check_if_they_are_errors() {
        let summarise = |stale: &str, severity: &str| {
            let toml = format!(
                "[baseline]\nstale = \"{stale}\"\n\n[contracts.f]\ntype = \"forbidden\"\nfrom = [\"a\"]\nto = [\"b\"]\nseverity = \"{severity}\"\n"
            );
            let project = Project::new(&values(&[("a/x.ts", "b/y.ts")]), &[], &toml)
                .baseline("f", &["a/x.ts -> b/y.ts", "a/gone.ts -> b/y.ts"]);
            let summary = ContractReport::summary(&[project.check("f")], &project.config.baseline);
            (summary.failed(), summary.counts[0].count)
        };
        assert_eq!(summarise("error", "error"), (true, 1));
        assert!(!summarise("warn", "error").0);
        assert!(!summarise("error", "warn").0);
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
        let baseline = BaselineConfig::default();
        let findings = report.findings(&project.graph, &mut sources, None, &baseline);
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
                "a/x.ts:2:1: error[ab]: `a` must not import `b`; remove this import",
                "error[ab]: `a` must not import `b` (via a/z.ts -> c/w.ts -> b/y.ts); \
                 remove this import, or one further along the chain",
            ]
        );
    }

    #[test]
    fn diagnostics_label_the_later_links_of_their_chain() {
        let project = Project::new(
            &values(&[("a/z.ts", "c/w.ts"), ("c/w.ts", "b/y.ts")]),
            &[],
            include_str!("fixtures/diagnostics.toml"),
        );
        let report = project.check("ab");
        let mut sources = SourceMap::new();
        // The imports are at bytes 100 and 200, the start of the second line of
        // their files.
        let importer = sources.add("a/z.ts", format!("{}\nimport './w';\n", " ".repeat(99)));
        let next = sources.add("c/w.ts", format!("{}\nimport './y';\n", " ".repeat(199)));
        let baseline = BaselineConfig::default();
        let diagnostics = report.diagnostics(&project.graph, &mut sources, None, &baseline);
        let [diagnostic] = diagnostics.as_slice() else { panic!("expected one diagnostic") };
        let block = |source, start, label: Option<&str>| {
            let span = Span::new(source, ByteRange::new(start, start + 10));
            ReportElement::CodeBlock(ReportCodeBlock { span, label: label.map(Into::into) })
        };
        assert_eq!(
            diagnostic.contents,
            [block(importer, 100, None), block(next, 200, Some("imports `b/y.ts`"))]
        );
        // The findings point at the first import alone.
        let findings = report.findings(&project.graph, &mut sources, None, &baseline);
        assert_eq!(findings[0].contents, diagnostic.contents[..1]);
    }

    #[test]
    fn chains_through_packages_show_the_package_between_the_files() {
        let project = Project::new(
            &values(&[
                ("cmd/main.go", "internal/db/"),
                ("internal/db/conn.go", "internal/cache/"),
                ("internal/cache/store.go", "node:os"),
            ]),
            &[],
            include_str!("fixtures/packages.toml"),
        );
        let report = project.check("through-packages");
        let Violation::Dependency { chains, .. } = &report.violations[0] else {
            panic!("expected a dependency");
        };
        let mut sources = SourceMap::new();
        // The imports are at bytes 100 and 200, the start of the second line of
        // their files.
        sources.add("cmd/main.go", format!("{}\nimport \"db\"\n", " ".repeat(99)));
        sources.add("internal/db/conn.go", format!("{}\nimport \"cache\"\n", " ".repeat(199)));
        let baseline = BaselineConfig::default();
        let mut builder = ReportBuilder {
            graph: &project.graph,
            sources: &mut sources,
            config: None,
            baseline: &baseline,
        };
        assert_eq!(
            builder.chain(&chains[0]),
            [
                "  cmd/main.go:2",
                "  -> internal/db",
                "  -> internal/db/conn.go:2",
                "  -> internal/cache",
            ]
            .join("\n")
        );
        assert_eq!(
            chain_summary(&project.graph, &chains[0]),
            "cmd/main.go -> internal/db -> internal/db/conn.go -> internal/cache"
        );

        let findings = report.findings(&project.graph, &mut sources, None, &baseline);
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
            ["cmd/main.go:2:1: error[through-packages]: `cmd` must not import `internal/cache` \
                 (via cmd/main.go -> internal/db -> internal/db/conn.go -> internal/cache); \
                 remove this import, or one further along the chain"]
        );
    }
}
