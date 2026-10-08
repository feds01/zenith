//! `zenith check`: check the architecture contracts.

use std::{io::Write, time::Instant};

use anyhow::Result;
use log::info;
use zenith_config::StaleLevel;
use zenith_contracts::Baseline;
use zenith_reporting::{RenderContext, failed, render};

use crate::{
    ExitStatus,
    cli::{CheckCommand, GlobalArgs},
    commands::compile_contracts,
    project::Project,
};

pub fn check(global: &GlobalArgs, args: CheckCommand) -> Result<ExitStatus> {
    let start = Instant::now();
    let project = Project::locate(global, true)?;
    let index = project.index()?;
    let contracts = compile_contracts(&project, &index, &args.contracts)?;

    let baseline = if args.no_baseline {
        None
    } else {
        let path = project.baseline_path();
        let baseline = Baseline::load(&path).map_err(anyhow::Error::msg)?;
        if baseline.is_some() {
            info!("using the baseline `{}`", path.display());
        }
        baseline
    };

    let all_ids: Vec<String> = project.config.contracts.keys().cloned().collect();
    let report = zenith_contracts::check(&index, &contracts, baseline.as_ref(), &all_ids);

    let stale_is_error = project.config.baseline.stale == StaleLevel::Error;
    let context = RenderContext {
        index: &index,
        report: &report,
        stale_is_error,
        show_unresolved: args.show_unresolved,
        show_timings: args.show_timings,
        duration: start.elapsed(),
    };
    let mut out = std::io::stdout().lock();
    render(args.format.into(), &context, &mut out)?;
    out.flush()?;

    Ok(if failed(&report, stale_is_error) { ExitStatus::Failure } else { ExitStatus::Success })
}
