//! `zenith check`.

use std::time::Instant;

use anyhow::Result;
use log::info;
use serde::Serialize;
use zenith_contracts::{
    Baseline, Contract, ContractJson, ContractReport, Exemptions, UnusedSuppression,
    UnusedSuppressionJson,
};
use zenith_pipeline::{FileProblemJson, StageKind, UnresolvedJson};
use zenith_reporting::{Format, Json, Output, Summary};

use crate::{
    ExitStatus,
    cli::{CheckCommand, GlobalArgs, ReportFormat},
    project,
};

/// Check the architecture contracts, and exit with [`ExitStatus::Failure`]
/// if a contract of `error` severity is broken, or has stale baseline entries
/// that are errors.
pub fn check(global: &GlobalArgs, args: CheckCommand) -> Result<ExitStatus> {
    // 1. Open the project, and build its graph.
    let mut session = project::open(global)?;
    let pipeline = project::run_until(&mut session, StageKind::Graph)?;

    // 2. Compile the contracts to check: every one, or those of `--contract`.
    let contracts = project::compile_contracts(&session, &args.contracts)?;

    // 3. Read the baseline, unless it is ignored.
    let baseline = match args.no_baseline {
        true => None,
        false => Baseline::load(&session.baseline_path())?,
    };
    if baseline.is_some() {
        info!("using the baseline `{}`", session.config.baseline.path);
    }

    // 4. Check the contracts in parallel, without the imports that suppressions and
    //    the baseline leave out, and find the suppressions that had no effect.
    let start = Instant::now();
    let exemptions =
        Exemptions { suppressions: &session.suppressions, baseline: baseline.as_ref() };
    let checked = Contract::check_all(&contracts, &session.graph, &session.workspace, &exemptions);
    let unused = UnusedSuppression::find(&session.config, &exemptions, &checked);
    let check_time = start.elapsed();

    // 5. Summarise every contract.
    let (graph, baseline) = (&session.graph, &session.config.baseline);
    let timings = pipeline
        .timings()
        .iter()
        .map(|(kind, time)| (kind.name(), *time))
        .chain([("check", check_time)])
        .collect();
    let summary = Summary {
        files: session.files.len(),
        imports: graph.detail_count(),
        timings: args.show_timings.then_some(timings),
        ..ContractReport::summary(&checked, baseline)
    };

    // 6. Show what they found, with their stale baseline entries, and the unused
    //    suppressions: as JSON, or as reports in full or a finding a line, then the
    //    summary.
    let (mut sources, config) = session.sources();
    let format = match args.format {
        ReportFormat::Pretty => Some(Format::Pretty),
        ReportFormat::Compact => Some(Format::Compact),
        ReportFormat::Github => Some(Format::Github),
        ReportFormat::Json => None,
    };
    match format {
        None => {
            let found = Found {
                contracts: checked
                    .iter()
                    .map(|report| report.json(graph, &mut sources, args.show_timings))
                    .collect(),
                unused_suppressions: unused
                    .iter()
                    .map(|unused| unused.json(&mut sources))
                    .collect(),
                unresolved: session
                    .unresolved
                    .iter()
                    .map(|import| import.json(&mut sources))
                    .collect(),
                file_errors: session
                    .file_errors
                    .iter()
                    .flat_map(|error| error.json(&mut sources))
                    .collect(),
            };
            println!("{}", Json { summary: &summary, found: &found });
        }
        Some(format) => {
            let mut reports = Vec::new();
            for report in &checked {
                reports.extend(match format {
                    Format::Pretty => report.reports(graph, &mut sources, config, baseline),
                    Format::Compact | Format::Github => {
                        report.findings(graph, &mut sources, config, baseline)
                    }
                });
            }
            reports.extend(unused.iter().map(|unused| unused.report(&mut sources)));
            if format == Format::Pretty {
                reports.extend(session.unresolved_report(&mut sources, args.show_unresolved));
            } else if args.show_unresolved {
                reports.extend(session.unresolved.iter().map(|import| import.report(&mut sources)));
            }
            println!(
                "{}",
                Output { sources: &sources, reports: &reports, summary: &summary, format }
            );
        }
    }

    // 7. Fail if a contract of `error` severity is broken, or has stale baseline
    //    entries that are errors.
    Ok(if summary.failed() { ExitStatus::Failure } else { ExitStatus::Success })
}

/// What `zenith check` found, as its JSON output lists it after the summary.
#[derive(Serialize)]
struct Found<'a> {
    /// The checked contracts, in the order of the configuration.
    contracts: Vec<ContractJson<'a>>,

    /// The suppressions that had no effect.
    unused_suppressions: Vec<UnusedSuppressionJson<'a>>,

    /// The imports that couldn't be resolved.
    unresolved: Vec<UnresolvedJson<'a>>,

    /// The errors in source files that couldn't be read or parsed in full.
    file_errors: Vec<FileProblemJson<'a>>,
}
