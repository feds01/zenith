//! `zenith check`.

use std::{io, time::Instant};

use anyhow::{Result, bail};
use zenith_contracts::Contract;
use zenith_pipeline::StageKind;
use zenith_reporting::{CheckResults, Sources};
use zenith_utils::highlight::colour_enabled;

use crate::{
    ExitStatus, Reported,
    cli::{CheckCommand, GlobalArgs, ReportFormat},
    project,
};

/// Check the architecture contracts, and exit with [`ExitStatus::Failure`]
/// if a contract of `error` severity is broken.
pub fn check(global: &GlobalArgs, args: CheckCommand) -> Result<ExitStatus> {
    let compact = match args.format {
        ReportFormat::Pretty => false,
        ReportFormat::Compact => true,
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
    let contracts = match Contract::compile_all(selected, &session.graph, &session.workspace) {
        Ok(contracts) => contracts,
        Err(errors) => {
            project::report_contract_errors(&session, &errors);
            return Err(Reported.into());
        }
    };

    // 4. Check them, in parallel.
    let start = Instant::now();
    let reports = Contract::check_all(&contracts, &session.graph, &session.workspace);
    let check_time = start.elapsed();

    // 5. Show what they found, and fail if a contract of `error` severity is
    //    broken.
    let timings: Vec<_> = pipeline
        .timings()
        .iter()
        .map(|(kind, time)| (kind.name(), *time))
        .chain([("check", check_time)])
        .collect();
    let results = CheckResults {
        session: &session,
        reports: &reports,
        show_unresolved: args.show_unresolved,
        timings: args.show_timings.then_some(timings.as_slice()),
    };
    let mut sources = Sources::new(&session.root);
    let mut out = io::stdout().lock();
    if compact {
        results.write_compact(&mut sources, &mut out)?;
    } else {
        results.write_pretty(&mut sources, colour_enabled(), &mut out)?;
    }
    Ok(if results.failed() { ExitStatus::Failure } else { ExitStatus::Success })
}
