//! `zenith check`.

use std::time::Instant;

use anyhow::{Result, bail};
use zenith_contracts::Contract;
use zenith_pipeline::StageKind;
use zenith_reporting::{Format, Output, Summary};

use crate::{
    ExitStatus, Reported,
    cli::{CheckCommand, GlobalArgs, ReportFormat},
    project, reporting,
};

/// Check the architecture contracts, and exit with [`ExitStatus::Failure`]
/// if a contract of `error` severity is broken.
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

    // 2. Pick the contracts to check: every one, or those of `--contract`.
    let configs = &session.config.contracts;
    for id in &args.contracts {
        if session.config.contract(id).is_none() {
            let known: Vec<_> = configs.iter().map(|config| format!("`{}`", config.id)).collect();
            match known.as_slice() {
                [] => bail!("there is no contract `{id}`; the configuration has no contracts"),
                known => {
                    bail!("there is no contract `{id}`; the contracts are {}", known.join(", "))
                }
            }
        }
    }
    let selected = configs
        .iter()
        .filter(|config| args.contracts.is_empty() || args.contracts.contains(&config.id));

    // 3. Compile them against the graph. Their errors are shown in the context of
    //    the configuration file.
    let (mut sources, config) = session.sources();
    let contracts = match Contract::compile_all(selected, &session.graph, &session.workspace) {
        Ok(contracts) => contracts,
        Err(errors) => {
            let reports: Vec<_> = errors.iter().map(|error| error.report(config)).collect();
            reporting::emit(&sources, &reports);
            return Err(Reported.into());
        }
    };

    // 4. Check them, in parallel.
    let start = Instant::now();
    let checked = Contract::check_all(&contracts, &session.graph, &session.workspace);
    let check_time = start.elapsed();

    // 5. Report what they found: in full, then the imports that couldn't be
    //    resolved, or a finding a line, with every unresolved import if asked.
    let graph = &session.graph;
    let mut reports = Vec::new();
    for report in &checked {
        reports.extend(match format {
            Format::Pretty => report.reports(graph, &mut sources, config),
            Format::Compact => report.findings(graph, &mut sources, config),
        });
    }
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
    //    `error` severity is broken.
    let timings = pipeline
        .timings()
        .iter()
        .map(|(kind, time)| (kind.name(), *time))
        .chain([("check", check_time)])
        .collect();
    let summary = Summary {
        entries: checked.iter().map(|report| report.summary_entry()).collect(),
        files: session.files.len(),
        imports: graph.detail_count(),
        timings: args.show_timings.then_some(timings),
    };
    println!("{}", Output { sources: &sources, reports: &reports, summary: &summary, format });
    Ok(if summary.failed() { ExitStatus::Failure } else { ExitStatus::Success })
}
