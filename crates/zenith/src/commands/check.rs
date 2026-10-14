//! `zenith check`.

use std::time::Instant;

use anyhow::{Result, bail};
use log::{info, warn};
use zenith_contracts::{
    Baseline, CheckJson, Contract, ContractReport, Exemptions, UnusedSuppression,
};
use zenith_pipeline::{Scope, Session, StageKind};
use zenith_reporting::{Format, Output, Summary};
use zenith_utils::path::{adjust_canonicalisation, relative_posix};

use crate::{
    ExitStatus,
    cli::{CheckCommand, GlobalArgs, ReportFormat},
    project,
};

/// Check the architecture contracts, and exit with [`ExitStatus::Failure`]
/// if a contract of `error` severity is broken, or has stale baseline entries
/// that are errors.
pub fn check(global: &GlobalArgs, args: CheckCommand) -> Result<ExitStatus> {
    // 1. Open the project, build its graph, and keep only the problems of the files
    //    under the paths, if any are given.
    let mut session = project::open(global)?;
    let pipeline = project::run_until(&mut session, StageKind::Graph)?;
    let scope = scope(&session, &args)?;
    session.narrow(&scope);

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
        report.narrow(graphs.of(report.config), &scope);
    }
    unused.retain(|unused| scope.contains(unused.file));
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

/// The files under the paths of `args`, or every file if there are none. A
/// path is relative to the working directory, and must be inside the root; it
/// need not exist, as a deleted file's importers may still involve it.
fn scope(session: &Session, args: &CheckCommand) -> Result<Scope> {
    let mut paths = Vec::new();
    for path in &args.paths {
        match relative_posix(&session.root, &adjust_canonicalisation(path)) {
            Some(name) => paths.push(name),
            None => bail!("`{}` is not inside the project root", path.display()),
        }
    }
    Ok(Scope::new(paths))
}
