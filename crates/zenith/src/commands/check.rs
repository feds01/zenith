//! `zenith check`.

use std::time::Instant;

use anyhow::{Result, bail};
use log::info;
use zenith_contracts::{Baseline, Contract, ContractReport, Exemptions, UnusedSuppression};
use zenith_pipeline::StageKind;
use zenith_reporting::{Format, Output, Summary};

use crate::{
    ExitStatus,
    cli::{CheckCommand, GlobalArgs, ReportFormat},
    project,
};

/// Check the architecture contracts, and exit with [`ExitStatus::Failure`]
/// if a contract of `error` severity is broken, or has stale baseline entries
/// that are errors.
pub fn check(global: &GlobalArgs, args: CheckCommand) -> Result<ExitStatus> {
    let format = match args.format {
        ReportFormat::Pretty => Format::Pretty,
        ReportFormat::Compact => Format::Compact,
        ReportFormat::Json => bail!("`--format json` is not supported yet"),
        ReportFormat::Github => bail!("`--format github` is not supported yet"),
    };

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

    // 5. Report what they found, with their stale baseline entries, and the unused
    //    suppressions: in full, then the imports that couldn't be resolved, or a
    //    finding a line, with every unresolved import if asked.
    let (mut sources, config) = session.sources();
    let (graph, baseline) = (&session.graph, &session.config.baseline);
    let mut reports = Vec::new();
    for report in &checked {
        reports.extend(match format {
            Format::Pretty => report.reports(graph, &mut sources, config, baseline),
            Format::Compact => report.findings(graph, &mut sources, config, baseline),
        });
    }
    reports.extend(unused.iter().map(|unused| unused.report(&mut sources)));
    match format {
        Format::Pretty => {
            reports.extend(session.unresolved_report(&mut sources, args.show_unresolved));
        }
        Format::Compact if args.show_unresolved => {
            reports.extend(session.unresolved.iter().map(|import| import.report(&mut sources)));
        }
        Format::Compact => {}
    }

    // 6. Show them with a summary of every contract, and fail if a contract of
    //    `error` severity is broken, or has stale baseline entries that are errors.
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
    println!("{}", Output { sources: &sources, reports: &reports, summary: &summary, format });
    Ok(if summary.failed() { ExitStatus::Failure } else { ExitStatus::Success })
}
