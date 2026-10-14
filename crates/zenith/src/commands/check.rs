//! `zenith check`.

use std::time::Instant;

use anyhow::Result;
use log::{info, warn};
use zenith_contracts::{
    Baseline, CheckJson, Contract, ContractReport, Exemptions, UnusedSuppression,
};
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
    // 1. Open the project, scoped to the paths if any are given, and build its
    //    graph.
    let mut session = project::open(global)?;
    session.scope_to(&args.paths)?;
    let pipeline = project::run_until(&mut session, StageKind::Graph)?;

    // 2. Compile the contracts to check: every one, or those of `--contract`.
    let contracts = project::compile_contracts(&session, &args.contracts)?;

    // 3. Read the baseline, unless it is ignored.
    let baseline = match args.no_baseline {
        true => None,
        false => Baseline::load(&session.baseline_path())?,
    };
    if let Some(baseline) = &baseline {
        let shown = &session.config.baseline.path;
        info!("using the baseline `{shown}`");
        if baseline.conflicted {
            warn!(
                "the baseline `{shown}` has merge conflict markers, so the entries of both sides \
                 count; `zenith baseline --prune` resolves them"
            );
        }
    }

    // 4. Check the contracts in parallel, without the imports that suppressions and
    //    the baseline leave out, and find the suppressions that had no effect. Keep
    //    only what involves the files under the paths.
    let start = Instant::now();
    let exemptions =
        Exemptions { suppressions: &session.suppressions, baseline: baseline.as_ref() };
    let graphs = session.graphs();
    let mut checked = Contract::check_all(&contracts, graphs, &session.workspace, &exemptions);
    let mut unused = UnusedSuppression::find(&session.config, &exemptions, &checked);
    for report in &mut checked {
        report.narrow(graphs.of(report.config), &session.scope);
    }
    unused.retain(|unused| session.scope.contains(unused.file));
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
            println!("{}", CheckJson::new(&session, &checked, &unused, &summary, &mut sources));
        }
        Some(format) => {
            let mut reports = Vec::new();
            for report in &checked {
                reports.extend(match format {
                    Format::Pretty => {
                        report.reports(graphs.of(report.config), &mut sources, config, baseline)
                    }
                    Format::Compact | Format::Github => {
                        report.findings(graphs.of(report.config), &mut sources, config, baseline)
                    }
                });
            }
            reports.extend(unused.iter().map(|unused| unused.report(&mut sources)));
            // Every format warns about the imports that couldn't be resolved, which
            // `--show-unresolved` lists: in the warning in full, or a line each.
            if format != Format::Pretty && args.show_unresolved {
                reports.extend(session.unresolved.iter().map(|import| import.report(&mut sources)));
            } else {
                reports.extend(session.unresolved_report(&mut sources, args.show_unresolved));
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
