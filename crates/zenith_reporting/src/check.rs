//! The output of `zenith check`, in the `pretty` and `compact` formats.

use std::{io, path::Path, time::Duration};

use zenith_config::Severity;
use zenith_contracts::{ContractReport, Violation};
use zenith_diagnostics::{Report, Reporter, SourceId};
use zenith_graph::Graph;
use zenith_pipeline::Session;
use zenith_utils::{
    highlight::{Colour, Modifier, highlight},
    printing::{plural, thousands},
};

use crate::{Sources, ViolationReporter, violations::import_span};

/// The width of the contract types in the summary, that of `encapsulation`.
const TYPE_WIDTH: usize = 13;

/// What `zenith check` found, to show.
pub struct CheckResults<'a> {
    /// The project the contracts were checked in: its graph, files and
    /// unresolved imports.
    pub session: &'a Session,

    /// The reports of the checked contracts, in the order of the
    /// configuration.
    pub reports: &'a [ContractReport<'a>],

    /// Whether to list every import that couldn't be resolved, rather than
    /// count them.
    pub show_unresolved: bool,

    /// How long each step took, by name, to show in the summary, or `None`
    /// to leave timings out.
    pub timings: Option<&'a [(&'static str, Duration)]>,
}

impl CheckResults<'_> {
    /// Whether the check fails: a contract of `error` severity is broken.
    pub fn failed(&self) -> bool {
        self.reports
            .iter()
            .any(|report| report.config.severity == Severity::Error && !report.is_kept())
    }

    /// Write a report with excerpts for every violation, a warning about the
    /// imports that couldn't be resolved, then a summary of every contract
    /// and the summary line.
    pub fn write_pretty(
        &self,
        sources: &mut Sources,
        colour: bool,
        out: &mut dyn io::Write,
    ) -> io::Result<()> {
        // 1. The reports of the violations, then of the unresolved imports.
        let config = self.config_source(sources);
        let mut reports = Vec::new();
        let mut reporter = ViolationReporter::new(self.graph(), sources, config);
        for report in self.reports {
            reports.extend(reporter.reports(report));
        }
        reports.extend(self.unresolved_report(sources));
        if !reports.is_empty() {
            writeln!(out, "{}\n", Reporter::new(sources.map(), &reports).with_colour(colour))?;
        }

        // 2. A line for every contract, then the summary line.
        writeln!(out, "{}", highlight(Modifier::Bold, "Contracts"))?;
        let width = self.reports.iter().map(|report| report.config.id.len()).max().unwrap_or(0);
        for report in self.reports {
            writeln!(out, "  {}", self.contract_line(report, width))?;
        }
        writeln!(out, "\n{}", self.summary_line())
    }

    /// Write a line for every finding, `path:line:col: level[contract]:
    /// message`, then the summary line.
    pub fn write_compact(&self, sources: &mut Sources, out: &mut dyn io::Write) -> io::Result<()> {
        let config = self.config_source(sources);
        let config_name = config.map(|id| sources.map().get(id).path.clone());
        for report in self.reports {
            let id = &report.config.id;
            let level = match report.config.severity {
                Severity::Error => "error",
                Severity::Warn => "warning",
            };
            for (location, message) in self.findings(sources, report) {
                let location = location.map(|location| format!("{location}: ")).unwrap_or_default();
                writeln!(out, "{location}{level}[{id}]: {message}")?;
            }
            for entry in report.unmatched_ignore_errors() {
                let location = match (&config_name, entry.range()) {
                    (Some(name), Some(range)) => match sources.position(name, range.start()) {
                        Some((line, column)) => format!("{name}:{line}:{column}: "),
                        None => String::new(),
                    },
                    _ => String::new(),
                };
                writeln!(
                    out,
                    "{location}{level}[{id}]: the ignored import `{entry}` matches no import"
                )?;
            }
        }
        if self.show_unresolved {
            for import in &self.session.unresolved {
                let (line, column) =
                    sources.position(&import.file, import.import.range.start()).unwrap_or_default();
                writeln!(
                    out,
                    "{}:{line}:{column}: warning: could not resolve `{}`: {}",
                    import.file, import.import.specifier, import.reason
                )?;
            }
        }
        writeln!(out, "{}", self.summary_line())
    }

    /// The project's graph.
    fn graph(&self) -> &Graph {
        &self.session.graph
    }

    /// Add the configuration file to `sources`, named by its path relative to
    /// the project root if it is in it, and return its id.
    fn config_source(&self, sources: &mut Sources) -> Option<SourceId> {
        let file = self.session.config.file.as_ref()?;
        let path = file.path.strip_prefix(&self.session.root).unwrap_or(&file.path);
        Some(sources.add(&posix(path), &file.contents))
    }

    /// The warning about the imports that couldn't be resolved, listing them
    /// if asked to, or `None` if there are none.
    fn unresolved_report(&self, sources: &mut Sources) -> Option<Report> {
        let unresolved = &self.session.unresolved;
        if unresolved.is_empty() {
            return None;
        }
        let title = format!("{} could not be resolved", plural(unresolved.len(), "import"));
        let mut report = Report::warning(title);
        if self.show_unresolved {
            let lines: Vec<_> = unresolved
                .iter()
                .map(|import| {
                    let (line, column) = sources
                        .position(&import.file, import.import.range.start())
                        .unwrap_or_default();
                    format!(
                        "  {}:{line}:{column}  `{}`  ({})",
                        import.file, import.import.specifier, import.reason
                    )
                })
                .collect();
            report.add_note(lines.join("\n"));
        } else {
            report.add_help(
                "use `--show-unresolved` to list them, or `[resolve] ignore_unresolved` to silence expected ones",
            );
        }
        Some(report)
    }

    /// The findings of `report`, one line each, with where they are as
    /// `path:line:col` if they have a place.
    fn findings(
        &self,
        sources: &mut Sources,
        report: &ContractReport,
    ) -> Vec<(Option<String>, String)> {
        let graph = self.graph();
        // A finding is where the specifier of its first import is.
        let location = |sources: &mut Sources, link: &zenith_contracts::Link| {
            let file = graph.name(link.importer);
            let import = link.imports.first()?;
            let start = import_span(sources.text(file)?, import).start();
            let (line, column) = sources.position(file, start)?;
            Some(format!("{file}:{line}:{column}"))
        };
        let encapsulation = report.config.kind.type_name() == "encapsulation";
        let mut findings = Vec::new();
        for violation in &report.violations {
            match violation {
                Violation::Dependency { importer, imported, chains } => {
                    for chain in chains {
                        let via = match chain.links.len() {
                            1 => String::new(),
                            _ => format!(" (via {})", chain_summary(graph, chain)),
                        };
                        let message = format!("`{importer}` must not import `{imported}`{via}");
                        findings.push((location(sources, &chain.links[0]), message));
                    }
                }
                Violation::IllegalImports { module, imports } => {
                    for link in imports {
                        let message = if encapsulation {
                            format!(
                                "`{}` is internal to `{module}`; import it through an entry point",
                                graph.name(link.imported)
                            )
                        } else {
                            format!(
                                "`{}` may not import the protected module `{module}`",
                                graph.name(link.importer)
                            )
                        };
                        findings.push((location(sources, link), message));
                    }
                }
                Violation::Cycle { scope, breakers } => {
                    for breaker in breakers {
                        let mut cycle = breaker.cycle.clone();
                        cycle.push(breaker.importer.clone());
                        let message = format!(
                            "dependency cycle {scope}: {}; remove `{}` -> `{}` to break it",
                            cycle.join(" -> "),
                            breaker.importer,
                            breaker.imported
                        );
                        let place =
                            breaker.imports.first().and_then(|link| location(sources, link));
                        findings.push((place, message));
                    }
                }
                Violation::UndeclaredModule { container, module } => {
                    let module = graph.name(*module);
                    let message = format!("`{module}` is not declared as a layer of `{container}`");
                    findings.push((None, message));
                }
            }
        }
        findings
    }

    /// The summary of one contract: whether it is kept, and how many
    /// findings break it, with the id padded to `width`.
    fn contract_line(&self, report: &ContractReport, width: usize) -> String {
        let config = report.config;
        let (mark, status) = if report.is_kept() {
            (highlight(Colour::Green, "✓"), "kept".to_string())
        } else {
            let violations = plural(report.finding_count(), "violation");
            match config.severity {
                Severity::Error => (highlight(Colour::Red, "✗"), format!("broken ({violations})")),
                Severity::Warn => {
                    (highlight(Colour::Yellow, "!"), format!("broken ({violations}, warning only)"))
                }
            }
        };
        let timing = match self.timings {
            Some(_) => highlight(Modifier::Dim, format!(" · {:.1?}", report.duration)),
            None => String::new(),
        };
        let kind = config.kind.type_name();
        format!("{mark} {:<width$}  {kind:<TYPE_WIDTH$} {status}{timing}", config.id)
    }

    /// `<kept> kept, <broken> broken[, <n> warnings] · <files> files,
    /// <imports> imports`, followed by the timings if they are shown.
    fn summary_line(&self) -> String {
        let broken = |severity| {
            let reports = self.reports.iter();
            reports.filter(|report| report.config.severity == severity && !report.is_kept()).count()
        };
        let kept = self.reports.iter().filter(|report| report.is_kept()).count();
        let mut line = format!("{kept} kept, {} broken", broken(Severity::Error));
        let warnings = broken(Severity::Warn);
        if warnings > 0 {
            line += &format!(", {}", plural(warnings, "warning"));
        }
        let files = self.session.files.len();
        let imports = self.graph().detail_count();
        line += &format!(
            " · {} {}, {} {}",
            thousands(files),
            if files == 1 { "file" } else { "files" },
            thousands(imports),
            if imports == 1 { "import" } else { "imports" }
        );
        if let Some(timings) = self.timings {
            let steps: Vec<_> =
                timings.iter().map(|(name, time)| format!("{name} {time:.1?}")).collect();
            line += &format!(" · {}", steps.join(", "));
        }
        line
    }
}

/// The nodes of `chain`, as `a.ts -> b.ts -> npm:c`, without the middle of a
/// chain longer than five nodes.
fn chain_summary(graph: &Graph, chain: &zenith_contracts::Chain) -> String {
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

/// `path` with `/` separators, as reports show paths.
fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use zenith_config::Config;
    use zenith_contracts::{Chain, Contract, Link};
    use zenith_graph::{GraphBuilder, ImportDetail, ImportKind, ModuleId};
    use zenith_pipeline::SourceFile;
    use zenith_utils::location::ByteRange;

    use super::*;

    /// A session of two files, `a/x.ts` importing `b/y.ts`, with the
    /// contracts of `toml`.
    fn session(toml: &str) -> Session {
        let mut builder = GraphBuilder::new();
        let [x, y] = ["a/x.ts", "b/y.ts"].map(|file| builder.add_file(file).unwrap());
        let detail = ImportDetail {
            range: ByteRange::new(0, 10),
            specifier: "../b/y".to_string(),
            kind: ImportKind::Value,
        };
        builder.add_import(x, y, detail);
        let mut session = Session::new(PathBuf::from("/repo"), Config::parse(toml).unwrap());
        session.graph = builder.finish();
        session.files = ["a/x.ts", "b/y.ts"]
            .map(|name| SourceFile {
                path: PathBuf::from("/repo").join(name),
                name: name.to_string(),
            })
            .to_vec();
        session
    }

    /// The summary line, and whether the check fails, of checking every
    /// contract of `toml`.
    fn summarise(toml: &str) -> (String, bool) {
        let session = session(toml);
        let contracts =
            Contract::compile_all(&session.config.contracts, &session.graph, &session.workspace)
                .unwrap();
        let reports = Contract::check_all(&contracts, &session.graph, &session.workspace);
        let results = CheckResults {
            session: &session,
            reports: &reports,
            show_unresolved: false,
            timings: None,
        };
        (results.summary_line(), results.failed())
    }

    #[test]
    fn only_broken_contracts_of_error_severity_fail_the_check() {
        let contract = |id: &str, from: &str, to: &str, severity: &str| {
            format!(
                "[contracts.{id}]\ntype = \"forbidden\"\nfrom = [\"{from}\"]\nto = [\"{to}\"]\nseverity = \"{severity}\"\n"
            )
        };
        let kept = contract("kept", "b", "a", "error");
        let warned = contract("warned", "a", "b", "warn");
        let broken = contract("broken", "a", "b", "error");

        assert_eq!(summarise(&kept), ("1 kept, 0 broken · 2 files, 1 import".to_string(), false));
        assert_eq!(
            summarise(&(kept.clone() + &warned)),
            ("1 kept, 0 broken, 1 warning · 2 files, 1 import".to_string(), false)
        );
        assert_eq!(
            summarise(&(kept + &warned + &broken)),
            ("1 kept, 1 broken, 1 warning · 2 files, 1 import".to_string(), true)
        );
    }

    #[test]
    fn long_chains_are_summarised_without_their_middle() {
        let mut builder = GraphBuilder::new();
        let files: Vec<_> = (0..6).map(|n| builder.add_file(&format!("{n}.ts")).unwrap()).collect();
        let graph = builder.finish();
        let ids: Vec<_> =
            files.iter().enumerate().map(|(n, _)| graph.id(&format!("{n}.ts")).unwrap()).collect();
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
}
