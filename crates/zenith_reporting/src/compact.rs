//! One line per finding: `path:line:col: <contract>: message`.

use std::io;

use zenith_config::Severity;
use zenith_utils::pluralise;

use crate::{RenderContext, findings};

pub(crate) fn render(context: &RenderContext, out: &mut dyn io::Write) -> io::Result<()> {
    for contract in &context.report.contracts {
        let level = match contract.severity {
            Severity::Error => "error",
            Severity::Warn => "warning",
        };
        for finding in findings(contract) {
            let location = match finding
                .link
                .and_then(|link| link.imports.first().map(|import| (link, import)))
            {
                Some((link, import)) => {
                    format!("{}:{}:{}: ", link.importer, import.line, import.column)
                }
                None => String::new(),
            };
            writeln!(out, "{location}{level}[{}]: {}", contract.id, finding.message)?;
        }
        for warning in &contract.warnings {
            writeln!(out, "warning[{}]: {warning}", contract.id)?;
        }
    }
    for unused in &context.report.unused_suppressions {
        writeln!(
            out,
            "{}:{}:{}: warning: unused suppression: {}",
            unused.file, unused.line, unused.column, unused.reason
        )?;
    }
    if context.show_unresolved {
        for unresolved in &context.index.unresolved {
            writeln!(
                out,
                "{}:{}:{}: warning: could not resolve `{}`: {}",
                unresolved.file,
                unresolved.line,
                unresolved.column,
                unresolved.specifier,
                unresolved.reason
            )?;
        }
    }

    let broken = context
        .report
        .contracts
        .iter()
        .filter(|c| c.status == zenith_contracts::Status::Broken)
        .count();
    let total = context.report.contracts.len();
    writeln!(out, "{broken} of {total} contract{} broken", pluralise!(total))
}
