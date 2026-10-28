//! `zenith check`.

mod json;

use std::time::Instant;

use anyhow::Result;
use json::CheckJson;
use log::warn;
use zenith_contracts::{ContractReport, Format, Output, Summary};
use zenith_diagnostics::message;
use zenith_pipeline::StageKind;
use zenith_workspace::Git;

use crate::{
    ExitStatus,
    cli::{CheckCommand, GlobalArgs, ReportFormat},
    project,
};

/// Check the architecture contracts, and exit with [`ExitStatus::Failure`]
/// if a contract of `error` severity is broken, or has stale baseline entries
/// that are errors.
pub fn check(global: &GlobalArgs, args: CheckCommand) -> Result<ExitStatus> {
    // 1. Open the project, scoped to the paths and to the files changed since the
    //    commit of `--since`, if any are given, and build its graph. Warn about the
    //    paths with no source file under them.
    let mut session = project::open(global)?;
    session.scope_to(&args.paths, args.since.as_deref())?;
    let pipeline = project::run_until(&mut session, StageKind::Graph)?;
    for path in session.paths_without_files(&args.paths) {
        warn!("{}", message!(cli_path_without_files, path = path.display()));
    }

    // 2. Compile the contracts to check: every one, or those of `--contract`.
    let contracts = project::compile_contracts(&session, &args.contracts)?;

    // 3. Read the baseline, unless it is ignored.
    let baseline = project::baseline(&session, args.no_baseline)?;

    // 4. Check the contracts in parallel, without the imports that suppressions and
    //    the baseline leave out, and find the suppressions that had no effect. Keep
    //    only what involves the files under the paths.
    let start = Instant::now();
    let graphs = project::graphs(&session);
    let (mut checked, mut unused) = project::check(&contracts, &session, baseline.as_ref());
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
    //    suppressions: as JSON or SARIF, or as reports in full or a finding a line,
    //    then the summary.
    let (mut sources, config) = session.sources();
    let format = match args.format {
        ReportFormat::Json => {
            println!("{}", CheckJson::new(&session, &checked, &unused, &summary, &mut sources));
            None
        }
        ReportFormat::Sarif => {
            // Code scanning reads paths as relative to the root of the repository.
            let prefix = Git::new(&session.root).root_prefix().unwrap_or_default();
            let mut sarif = ContractReport::sarif(
                &checked,
                &unused,
                graphs,
                baseline,
                &mut sources,
                &config,
                &prefix,
            );
            // The imports that couldn't be resolved are problems of the run.
            for import in &session.unresolved {
                sarif.add_notification(&import.report(&mut sources), &sources);
            }
            println!("{sarif}");
            None
        }
        ReportFormat::Pretty => Some(Format::Pretty),
        ReportFormat::Compact => Some(Format::Compact),
        ReportFormat::Github => Some(Format::Github),
    };
    if let Some(format) = format {
        let mut reports = Vec::new();
        for report in &checked {
            reports.extend(match format {
                Format::Pretty => {
                    report.reports(graphs.of(report.config), &mut sources, &config, baseline)
                }
                Format::Compact | Format::Github => {
                    report.findings(graphs.of(report.config), &mut sources, &config, baseline)
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
        println!("{}", Output { sources: &sources, reports: &reports, summary: &summary, format });
    }

    // 7. Fail if a contract of `error` severity is broken, or has stale baseline
    //    entries that are errors.
    Ok(if summary.failed() { ExitStatus::Failure } else { ExitStatus::Success })
}
