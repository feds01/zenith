//! `zenith lsp`.

use std::{env, panic, path};

use anyhow::{Error, Result};
use log::error;
use zenith_contracts::Contract;
use zenith_lsp::{Failure, Found, Server};
use zenith_pipeline::StageKind;
use zenith_utils::highlight::set_colour_enabled;

use crate::{ExitStatus, Reported, cli::GlobalArgs, crash::report_crash, project};

/// Run a language server on stdin and stdout, which publishes what the
/// contracts find as diagnostics in the editor that started it, until the
/// editor tells it to exit. It fails if the editor did not shut it down
/// first, as the protocol says.
pub fn lsp(global: &GlobalArgs) -> Result<ExitStatus> {
    // 1. Write the diagnostics, and the logs, without colour. A check that panics
    //    is reported, and fails, rather than ending the server.
    set_colour_enabled(false);
    panic::set_hook(Box::new(report_crash));

    // 2. Start the server, and find the project as `zenith check` would in the
    //    folder that the editor opened, with the paths of the options as given.
    let server = Server::stdio()?;
    let mut global = global.clone();
    if let Some(folder) = server.folder() {
        for option in [&mut global.config, &mut global.root].into_iter().flatten() {
            *option = path::absolute(&option)?;
        }
        env::set_current_dir(folder)?;
    }

    // 3. Publish what checking the project finds, now and on every save. A check
    //    that fails says why on stderr too.
    let shut_down = server.run(|| check(&global).map_err(failure))?;
    Ok(if shut_down { ExitStatus::Success } else { ExitStatus::Failure })
}

/// Check every contract of the project, as `zenith check` does, and return
/// the diagnostics of what they find, and of the suppressions that had no
/// effect.
fn check(global: &GlobalArgs) -> Result<Found> {
    // 1. Open the project, and build its graph.
    let mut session = project::open(global)?;
    project::run_until(&mut session, StageKind::Graph)?;

    // 2. Compile every contract, and read the baseline.
    let contracts = project::compile_contracts(&session, &[])?;
    let baseline = project::baseline(&session, false)?;

    // 3. Check the contracts, without the imports that suppressions and the
    //    baseline leave out, and find the suppressions that had no effect.
    let (checked, unused) = Contract::check_session(&contracts, &session, baseline.as_ref());

    // 4. Make the diagnostics of what they found.
    let graphs = session.graphs();
    let (mut sources, config) = session.sources();
    let baseline = &session.config.baseline;
    let reports = checked
        .iter()
        .flat_map(|report| {
            report.diagnostics(graphs.of(report.config), &mut sources, config, baseline)
        })
        .collect();
    let unnecessary = unused.iter().map(|unused| unused.report(&mut sources)).collect();
    Ok(Found { sources, reports, unnecessary })
}

/// What stopped a check, shown on stderr: its reports, such as errors in the
/// configuration file, or else its message.
fn failure(error: Error) -> Failure {
    match Reported::emit(error) {
        Ok(Reported { sources, reports }) => Failure::Reports(sources, reports),
        Err(error) => {
            error!("{error:#}");
            Failure::Message(format!("{error:#}"))
        }
    }
}
