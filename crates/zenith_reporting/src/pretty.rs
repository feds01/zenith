//! The default, human-readable output: an `annotate-snippets` report per
//! violation, followed by a summary of every contract.

use std::io;

use annotate_snippets::{AnnotationKind, Element, Group, Level, Renderer, Snippet};
use zenith_config::Severity;
use zenith_contracts::{Chain, ContractReport, Link, Status, Violation};
use zenith_utils::{
    format_duration,
    highlight::{Colour, Modifier, colour_enabled, highlight},
    pluralise,
    printing::thousands,
};

use crate::{
    RenderContext,
    sources::{Sources, specifier_span},
};

/// How many chains of one violation are listed.
const MAX_CHAINS: usize = 5;
/// How many code excerpts one report shows.
const MAX_SNIPPETS: usize = 3;
/// How many illegal imports one report lists.
const MAX_IMPORTS: usize = 10;
/// How many cycle breakers one report lists.
const MAX_BREAKERS: usize = 5;

pub(crate) fn render(context: &RenderContext, out: &mut dyn io::Write) -> io::Result<()> {
    let renderer = if colour_enabled() { Renderer::styled() } else { Renderer::plain() };
    let sources = Sources::new(&context.index.root);
    let mut printer = Printer { renderer, sources, out };

    for contract in &context.report.contracts {
        printer.contract(contract, context.stale_is_error)?;
    }
    printer.suppressions(context)?;
    printer.diagnostics(context)?;
    printer.summary(context)
}

struct Printer<'a> {
    renderer: Renderer,
    sources: Sources<'a>,
    out: &'a mut dyn io::Write,
}

impl Printer<'_> {
    fn emit(&mut self, group: Group) -> io::Result<()> {
        writeln!(self.out, "{}\n", self.renderer.render(&[group]))
    }

    /// A code excerpt underlining one import of a link.
    fn snippet<'s>(&self, source: &'s str, link: &'s Link, label: String) -> Option<Element<'s>> {
        let import = link.imports.first()?;
        let span = specifier_span(source, import);
        Some(
            Snippet::source(source)
                .path(link.importer.as_str())
                .fold(true)
                .annotation(AnnotationKind::Primary.span(span).label(label))
                .into(),
        )
    }

    fn contract(&mut self, contract: &ContractReport, stale_is_error: bool) -> io::Result<()> {
        let level = match contract.severity {
            Severity::Error => Level::ERROR,
            Severity::Warn => Level::WARNING,
        };

        for violation in &contract.violations {
            self.violation(level.clone(), contract, violation)?;
        }
        for error in &contract.errors {
            self.emit(
                level
                    .clone()
                    .primary_title(error.as_str())
                    .id(contract.id.as_str())
                    .elements(Vec::<Element>::new()),
            )?;
        }
        if contract.status == Status::Broken
            && let Some(guidance) = &contract.guidance
        {
            let guidance = guidance.trim();
            let title = format!("how to fix `{}`", contract.id);
            self.emit(
                Level::HELP
                    .primary_title(title)
                    .elements([Level::HELP.no_name().message(guidance)]),
            )?;
        }
        for warning in &contract.warnings {
            self.emit(
                Level::WARNING
                    .primary_title(warning.as_str())
                    .id(contract.id.as_str())
                    .elements(Vec::<Element>::new()),
            )?;
        }
        if !contract.stale_baseline.is_empty() {
            let count = contract.stale_baseline.len();
            let title = format!(
                "{count} stale baseline entr{} for `{}`",
                if count == 1 { "y" } else { "ies" },
                contract.id
            );
            let level = if stale_is_error && contract.severity == Severity::Error {
                level.clone()
            } else {
                Level::WARNING
            };
            let listed = contract
                .stale_baseline
                .iter()
                .map(|entry| format!("  {entry}"))
                .collect::<Vec<_>>()
                .join("\n");
            self.emit(level.clone().primary_title(title).id(contract.id.as_str()).elements([
                Level::NOTE.message(format!("these imports no longer exist:\n{listed}")),
                Level::HELP.message("run `zenith baseline --prune` to remove them"),
            ]))?;
        }
        Ok(())
    }

    fn violation(
        &mut self,
        level: Level<'static>,
        contract: &ContractReport,
        violation: &Violation,
    ) -> io::Result<()> {
        let id = contract.id.as_str();
        match violation {
            Violation::Dependency { importer, imported, chains } => {
                let title = format!("`{importer}` must not import `{imported}`");
                let sources: Vec<(usize, Option<String>)> = chains
                    .iter()
                    .take(MAX_SNIPPETS)
                    .enumerate()
                    .map(|(index, chain)| {
                        (
                            index,
                            chain.links.first().and_then(|link| self.sources.get(&link.importer)),
                        )
                    })
                    .collect();

                let mut elements: Vec<Element> = Vec::new();
                for (index, source) in &sources {
                    let chain = &chains[*index];
                    let Some(source) = source else { continue };
                    let link = &chain.links[0];
                    let label = if chain.links.len() == 1 {
                        format!("imports `{}`", link.imported)
                    } else {
                        format!("starts a chain of {} imports", chain.links.len())
                    };
                    if let Some(snippet) = self.snippet(source, link, label) {
                        elements.push(snippet);
                    }
                }
                let indirect: Vec<&Chain> =
                    chains.iter().filter(|chain| chain.links.len() > 1).collect();
                for (index, chain) in indirect.iter().take(MAX_CHAINS).enumerate() {
                    let heading = if indirect.len() == 1 {
                        "import chain:".to_string()
                    } else {
                        format!("import chain {} of {}:", index + 1, indirect.len())
                    };
                    elements.push(
                        Level::NOTE.message(format!("{heading}\n{}", chain_text(chain))).into(),
                    );
                }
                if indirect.len() > MAX_CHAINS {
                    elements.push(
                        Level::NOTE
                            .message(format!("and {} more chains", indirect.len() - MAX_CHAINS))
                            .into(),
                    );
                }
                let direct = chains.len() - indirect.len();
                if direct > MAX_SNIPPETS {
                    elements.push(Level::NOTE.message(format!(
                        "{direct} direct imports in total; use `--format compact` to list them all"
                    )).into());
                }
                self.emit(level.clone().primary_title(title).id(id).elements(elements))
            }
            Violation::IllegalImports { module, imports } => {
                let (title, label): (String, fn(&Link) -> String) = if contract.kind
                    == "encapsulation"
                {
                    (format!("`{module}` may only be imported through its entry points"), |link| {
                        format!("imports `{}`, which is internal", link.imported)
                    })
                } else {
                    (format!("`{module}` may only be imported by its allowed importers"), |link| {
                        format!("imports `{}`", link.imported)
                    })
                };
                let shown: Vec<(&Link, Option<String>)> = imports
                    .iter()
                    .take(MAX_IMPORTS)
                    .map(|link| (link, self.sources.get(&link.importer)))
                    .collect();
                let mut elements: Vec<Element> = Vec::new();
                for (link, source) in &shown {
                    if let Some(source) = source
                        && let Some(snippet) = self.snippet(source, link, label(link))
                    {
                        elements.push(snippet);
                    }
                }
                if imports.len() > MAX_IMPORTS {
                    let more = imports.len() - MAX_IMPORTS;
                    elements.push(
                        Level::NOTE
                            .message(format!("and {more} more import{}", pluralise!(more)))
                            .into(),
                    );
                }
                self.emit(level.clone().primary_title(title).id(id).elements(elements))
            }
            Violation::Cycle { scope, breakers } => {
                let title = format!("dependency cycle in `{scope}`");
                let count = breakers.len();
                let mut notes = vec![format!(
                    "it could be made acyclic by removing {count} {}:",
                    pluralise_word("dependency", "dependencies", count)
                )];
                for breaker in breakers.iter().take(MAX_BREAKERS) {
                    let imports = breaker.imports.len();
                    notes.push(format!(
                        "  {} -> {} ({imports} import{})",
                        breaker.importer,
                        breaker.imported,
                        pluralise!(imports)
                    ));
                }
                if count > MAX_BREAKERS {
                    notes.push(format!("  and {} more", count - MAX_BREAKERS));
                }

                let first = breakers.first().and_then(|breaker| breaker.imports.first());
                let source = first.and_then(|link| self.sources.get(&link.importer));
                let mut elements: Vec<Element> = Vec::new();
                if let (Some(link), Some(source)) = (first, &source) {
                    let breaker = &breakers[0];
                    let label = format!("closes the cycle {}", breaker.cycle.join(" -> "));
                    if let Some(snippet) = self.snippet(source, link, label) {
                        elements.push(snippet);
                    }
                }
                elements.push(Level::NOTE.message(notes.join("\n")).into());
                self.emit(level.clone().primary_title(title).id(id).elements(elements))
            }
            Violation::UndeclaredModule { container, module } => {
                let title = format!("`{module}` is not declared as a layer of `{container}`");
                self.emit(level.clone().primary_title(title).id(id).elements([Level::HELP.message(
                    "this contract is exhaustive: declare it in `layers`, or add it to `exhaustive_ignores`",
                )]))
            }
        }
    }

    fn suppressions(&mut self, context: &RenderContext) -> io::Result<()> {
        for unused in &context.report.unused_suppressions {
            let title = format!("unused suppression: {}", unused.reason);
            let source = self.sources.get(&unused.file);
            let mut elements: Vec<Element> = Vec::new();
            if let Some(source) = &source {
                let start =
                    line_offset(source, unused.line) + (unused.column as usize).saturating_sub(1);
                let end = source[start..].find('\n').map_or(source.len(), |offset| start + offset);
                elements.push(
                    Snippet::source(source.as_str())
                        .path(unused.file.as_str())
                        .fold(true)
                        .annotation(AnnotationKind::Primary.span(start..end.max(start + 1)))
                        .into(),
                );
            }
            self.emit(Level::WARNING.primary_title(title).elements(elements))?;
        }
        Ok(())
    }

    fn diagnostics(&mut self, context: &RenderContext) -> io::Result<()> {
        let index = context.index;
        if !index.unresolved.is_empty() {
            let count = index.unresolved.len();
            let title = format!("{count} import{} could not be resolved", pluralise!(count));
            let element = if context.show_unresolved {
                let listed = index
                    .unresolved
                    .iter()
                    .map(|u| {
                        format!(
                            "  {}:{}:{}  `{}`  ({})",
                            u.file, u.line, u.column, u.specifier, u.reason
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Level::NOTE.message(listed)
            } else {
                Level::HELP.message("use `--show-unresolved` to list them, or `[resolve] ignore_unresolved` to silence expected ones")
            };
            self.emit(Level::WARNING.primary_title(title).elements([element]))?;
        }
        if !index.parse_errors.is_empty() {
            let count = index.parse_errors.len();
            let title = format!("{count} file{} could not be parsed cleanly", pluralise!(count));
            let listed = index
                .parse_errors
                .iter()
                .take(10)
                .map(|(file, error)| format!("  {file}: {error}"))
                .collect::<Vec<_>>()
                .join("\n");
            self.emit(Level::WARNING.primary_title(title).elements([
                Level::NOTE.message(listed),
                Level::NOTE.message("imports recovered from these files are still checked"),
            ]))?;
        }
        Ok(())
    }

    fn summary(&mut self, context: &RenderContext) -> io::Result<()> {
        let contracts = &context.report.contracts;
        let width = contracts.iter().map(|c| c.id.len()).max().unwrap_or(0);

        writeln!(self.out, "{}", highlight(Modifier::Bold, "Contracts"))?;
        for contract in contracts {
            let failing = contract.fails(context.stale_is_error);
            let (mark, status) = match (contract.status, failing) {
                (Status::Kept, false) if contract.stale_baseline.is_empty() => {
                    (highlight(Colour::Green, "✓"), "kept".to_string())
                }
                (Status::Kept, _) => {
                    (highlight(Colour::Yellow, "!"), "kept, with a stale baseline".to_string())
                }
                (Status::Broken, _) => {
                    let count = contract.finding_count();
                    let warn =
                        if contract.severity == Severity::Warn { ", warning only" } else { "" };
                    let mark = if failing {
                        highlight(Colour::Red, "✗")
                    } else {
                        highlight(Colour::Yellow, "!")
                    };
                    (mark, format!("broken ({count} violation{}{warn})", pluralise!(count)))
                }
            };
            let mut extras = Vec::new();
            if contract.baselined_imports > 0 {
                extras.push(format!("{} baselined", contract.baselined_imports));
            }
            if contract.ignored_imports > 0 {
                extras.push(format!("{} ignored", contract.ignored_imports));
            }
            if contract.suppressed_imports > 0 {
                extras.push(format!("{} suppressed", contract.suppressed_imports));
            }
            if context.show_timings {
                extras.push(format_duration(contract.duration));
            }
            let extras = if extras.is_empty() {
                String::new()
            } else {
                highlight(Modifier::Dim, format!(" · {}", extras.join(", ")))
            };
            writeln!(
                self.out,
                "  {mark} {id:<width$}  {kind:<13} {status}{extras}",
                id = contract.id,
                kind = contract.kind,
            )?;
        }

        let kept = contracts.iter().filter(|c| c.status == Status::Kept).count();
        let broken = contracts
            .iter()
            .filter(|c| c.status == Status::Broken && c.severity == Severity::Error)
            .count();
        let warned = contracts
            .iter()
            .filter(|c| c.status == Status::Broken && c.severity == Severity::Warn)
            .count();
        let stale: usize = contracts.iter().map(|c| c.stale_baseline.len()).sum();
        let stats = &context.index.stats;

        let mut line = format!("\n{kept} kept, {broken} broken");
        if warned > 0 {
            line.push_str(&format!(", {warned} warning{}", pluralise!(warned)));
        }
        if stale > 0 {
            line.push_str(&format!(
                " · {stale} stale baseline entr{}",
                if stale == 1 { "y" } else { "ies" }
            ));
        }
        line.push_str(&format!(
            " · {} file{}, {} import{}",
            thousands(stats.files),
            pluralise!(stats.files),
            thousands(stats.imports),
            pluralise!(stats.imports)
        ));
        if context.show_timings {
            line.push_str(&format!(
                " · walk {}, parse {}, graph {}, total {}",
                format_duration(stats.walk_duration),
                format_duration(stats.parse_duration),
                format_duration(stats.graph_duration),
                format_duration(context.duration)
            ));
        }
        writeln!(self.out, "{line}")
    }
}

/// A chain as lines of `file:line` hops.
fn chain_text(chain: &Chain) -> String {
    let mut lines = Vec::new();
    for (index, link) in chain.links.iter().enumerate() {
        let arrow = if index == 0 { "  " } else { "  -> " };
        lines.push(format!("{arrow}{}{}", link.importer, link_lines(link)));
        if index == 0 {
            for head in &chain.extra_heads {
                lines.push(format!("   & {}{}", head.importer, link_lines(head)));
            }
        }
    }
    if let Some(last) = chain.links.last() {
        lines.push(format!("  -> {}", last.imported));
        for tail in &chain.extra_tails {
            lines.push(format!("   & {}", tail.imported));
        }
    }
    lines.join("\n")
}

/// `:3` or `:3,7`, plus the import kind when no import is a plain value
/// import, e.g. `:12 (dynamic)`.
fn link_lines(link: &Link) -> String {
    let lines: Vec<String> = link.imports.iter().map(|import| import.line.to_string()).collect();
    let kinds: Vec<&str> = link.imports.iter().map(|import| import.kind.name()).collect();
    let kind = match kinds.as_slice() {
        [first, rest @ ..] if *first != "value" && rest.iter().all(|k| k == first) => {
            format!(" ({first})")
        }
        _ => String::new(),
    };
    if lines.is_empty() { kind } else { format!(":{}{kind}", lines.join(",")) }
}

fn pluralise_word<'w>(one: &'w str, many: &'w str, count: usize) -> &'w str {
    if count == 1 { one } else { many }
}

fn line_offset(source: &str, line: u32) -> usize {
    if line <= 1 {
        return 0;
    }
    source.match_indices('\n').nth(line as usize - 2).map_or(source.len(), |(offset, _)| offset + 1)
}
