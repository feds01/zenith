//! Rendering check results: `pretty` (the default), `compact`, `json` and
//! `github` (workflow commands for GitHub Actions annotations).

mod compact;
mod github;
mod json;
mod pretty;
mod sources;

use std::{io, time::Duration};

use zenith_contracts::{CheckReport, ContractReport, Link, Violation};
use zenith_index::Index;

/// The output formats of `zenith check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Pretty,
    Compact,
    Json,
    Github,
}

/// Everything a renderer needs.
pub struct RenderContext<'a> {
    pub index: &'a Index,
    pub report: &'a CheckReport,
    /// Whether stale baseline entries fail the check.
    pub stale_is_error: bool,
    /// List every unresolved import, rather than just counting them.
    pub show_unresolved: bool,
    /// Show how long each phase and contract took.
    pub show_timings: bool,
    /// The total run time, shown with `show_timings`.
    pub duration: Duration,
}

/// Render the results in the given format.
pub fn render(
    format: OutputFormat,
    context: &RenderContext,
    out: &mut dyn io::Write,
) -> io::Result<()> {
    match format {
        OutputFormat::Pretty => pretty::render(context, out),
        OutputFormat::Compact => compact::render(context, out),
        OutputFormat::Json => json::render(context, out),
        OutputFormat::Github => github::render(context, out),
    }
}

/// Whether the check failed.
pub fn failed(report: &CheckReport, stale_is_error: bool) -> bool {
    report.contracts.iter().any(|contract| contract.fails(stale_is_error))
}

/// A one-line message and the location it points at, used by the line-based
/// formats.
pub(crate) struct Finding<'r> {
    pub message: String,
    pub link: Option<&'r Link>,
}

/// Flatten a contract's violations into line-based findings.
pub(crate) fn findings(contract: &ContractReport) -> Vec<Finding<'_>> {
    let mut findings = Vec::new();
    for violation in &contract.violations {
        match violation {
            Violation::Dependency { importer, imported, chains } => {
                for chain in chains {
                    let via = if chain.links.len() > 1 {
                        format!(" (via {})", chain_summary(&chain.links))
                    } else {
                        String::new()
                    };
                    findings.push(Finding {
                        message: format!("`{importer}` must not import `{imported}`{via}"),
                        link: chain.links.first(),
                    });
                }
            }
            Violation::IllegalImports { module, imports } => {
                for link in imports {
                    findings.push(Finding {
                        message: match contract.kind {
                            "encapsulation" => format!(
                                "`{}` is internal to `{module}`; import it through an entry point",
                                link.imported
                            ),
                            _ => format!(
                                "`{}` may not import the protected module `{module}`",
                                link.importer
                            ),
                        },
                        link: Some(link),
                    });
                }
            }
            Violation::Cycle { scope, breakers } => {
                for breaker in breakers {
                    findings.push(Finding {
                        message: format!(
                            "dependency cycle in `{scope}`: {}; remove `{}` -> `{}` to break it",
                            breaker.cycle.join(" -> "),
                            breaker.importer,
                            breaker.imported
                        ),
                        link: breaker.imports.first(),
                    });
                }
            }
            Violation::UndeclaredModule { container, module } => findings.push(Finding {
                message: format!("`{module}` is not declared as a layer of `{container}`"),
                link: None,
            }),
        }
    }
    for error in &contract.errors {
        findings.push(Finding { message: error.clone(), link: None });
    }
    for entry in &contract.stale_baseline {
        findings.push(Finding {
            message: format!("stale baseline entry `{entry}`; run `zenith baseline --prune`"),
            link: None,
        });
    }
    findings
}

/// `a.ts -> b.ts -> c.ts`, eliding the middle of long chains.
fn chain_summary(links: &[Link]) -> String {
    let mut nodes: Vec<&str> = links.iter().map(|link| link.importer.as_str()).collect();
    if let Some(last) = links.last() {
        nodes.push(&last.imported);
    }
    if nodes.len() > 5 {
        let elided = nodes.len() - 4;
        format!(
            "{} -> {} -> ({elided} more) -> {} -> {}",
            nodes[0],
            nodes[1],
            nodes[nodes.len() - 2],
            nodes[nodes.len() - 1]
        )
    } else {
        nodes.join(" -> ")
    }
}
