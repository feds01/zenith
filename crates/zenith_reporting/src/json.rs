//! Machine-readable output: the full report as JSON.

use std::io;

use serde::Serialize;
use zenith_contracts::{CheckReport, Status};
use zenith_index::UnresolvedImport;

use crate::{RenderContext, failed};

/// The version of the JSON output schema. Bump it on breaking changes.
const JSON_VERSION: u32 = 1;

#[derive(Serialize)]
struct JsonOutput<'a> {
    version: u32,
    passed: bool,
    summary: Summary,
    #[serde(flatten)]
    report: &'a CheckReport,
    unresolved: Vec<Unresolved<'a>>,
    parse_errors: Vec<ParseError<'a>>,
}

#[derive(Serialize)]
struct Summary {
    contracts: usize,
    kept: usize,
    broken: usize,
    files: usize,
    imports: usize,
}

#[derive(Serialize)]
struct Unresolved<'a> {
    file: &'a str,
    line: u32,
    column: u32,
    specifier: &'a str,
    reason: &'a str,
}

#[derive(Serialize)]
struct ParseError<'a> {
    file: &'a str,
    error: &'a str,
}

pub(crate) fn render(context: &RenderContext, out: &mut dyn io::Write) -> io::Result<()> {
    let report = context.report;
    let kept = report.contracts.iter().filter(|c| c.status == Status::Kept).count();
    let output = JsonOutput {
        version: JSON_VERSION,
        passed: !failed(report, context.stale_is_error),
        summary: Summary {
            contracts: report.contracts.len(),
            kept,
            broken: report.contracts.len() - kept,
            files: context.index.stats.files,
            imports: context.index.stats.imports,
        },
        report,
        unresolved: context.index.unresolved.iter().map(unresolved).collect(),
        parse_errors: context
            .index
            .parse_errors
            .iter()
            .map(|(file, error)| ParseError { file, error })
            .collect(),
    };
    serde_json::to_writer_pretty(&mut *out, &output).map_err(io::Error::other)?;
    writeln!(out)
}

fn unresolved(import: &UnresolvedImport) -> Unresolved<'_> {
    Unresolved {
        file: &import.file,
        line: import.line,
        column: import.column,
        specifier: &import.specifier,
        reason: &import.reason,
    }
}
