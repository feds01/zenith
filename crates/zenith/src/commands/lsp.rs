//! `zenith lsp`.

use std::{
    env, panic,
    path::{self, Path},
};

use anyhow::{Error, Result};
use log::error;
use zenith_lsp::{Changes, Failure, Found, Server};
use zenith_pipeline::{Pipeline, Session, StageKind};
use zenith_utils::{fs::adjust_canonicalisation, highlight::set_colour_enabled};

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

    // 3. Publish what checking the project finds, now and on every save, with the
    //    graph of the last check updated with what changed since, if it can be. A
    //    check that fails says why on stderr too.
    let folder = server.folder().map(adjust_canonicalisation);
    let mut built = None;
    let shut_down = server
        .run(|changes| check(&global, folder.as_deref(), &mut built, &changes).map_err(failure))?;
    Ok(if shut_down { ExitStatus::Success } else { ExitStatus::Failure })
}

/// The graph of the project, and the pipeline that built it, as a check
/// leaves them for the next to update.
struct Built {
    /// The project, and what the pipeline found in it.
    session: Session,

    /// The pipeline that built the graph.
    pipeline: Pipeline,

    /// Whether the project is in the folder that the editor opened, the only
    /// one whose changes it reports.
    watched: bool,
}

impl Built {
    /// Open the project, and build its graph, in the canonical `folder` that
    /// the editor opened, if it said.
    fn new(global: &GlobalArgs, folder: Option<&Path>) -> Result<Self> {
        let mut session = project::open(global)?;
        let pipeline = project::run_until(&mut session, StageKind::Graph)?;
        let watched = folder.is_some_and(|folder| session.is_within(folder));
        Ok(Self { session, pipeline, watched })
    }

    /// The graph updated with `changes`, or `None` if they may change more
    /// than what source files import, so that it must be built anew. The
    /// changes that the editor reports are all there are only in the folder
    /// it opened.
    fn update(mut self, changes: &Changes) -> Result<Option<Self>> {
        let Changes::Files(paths) = changes else {
            return Ok(None);
        };
        if !self.watched {
            return Ok(None);
        }
        let Some(changed) = self.session.changed_files(paths) else {
            return Ok(None);
        };
        project::update(&mut self.session, &mut self.pipeline, &changed)?;
        Ok(Some(self))
    }
}

/// Check every contract of the project, as `zenith check` does, and return
/// the diagnostics of what they find, and of the suppressions that had no
/// effect. The graph that the last check `built` is updated with the
/// `changes` since, if it can be, and kept for the next check. The editor
/// opened the canonical `folder`, if it said.
fn check(
    global: &GlobalArgs,
    folder: Option<&Path>,
    built: &mut Option<Built>,
    changes: &Changes,
) -> Result<Found> {
    // 1. Update the graph of the last check, or else open the project and build its
    //    graph. A graph that fails to update or build is not kept.
    let updated = match built.take() {
        Some(last) => last.update(changes)?,
        None => None,
    };
    let session = match updated {
        Some(updated) => &built.insert(updated).session,
        None => &built.insert(Built::new(global, folder)?).session,
    };

    // 2. Compile every contract, and read the baseline.
    let contracts = project::compile_contracts(session, &[])?;
    let baseline = project::baseline(session, false)?;

    // 3. Check the contracts, without the imports that suppressions and the
    //    baseline leave out, and find the suppressions that had no effect.
    let (checked, unused) = project::check(&contracts, session, baseline.as_ref());

    // 4. Make the diagnostics of what they found.
    let graphs = project::graphs(session);
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
