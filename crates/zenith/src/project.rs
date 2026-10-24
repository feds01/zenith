//! Finding the project and its configuration, and running the pipeline on
//! it.

use std::{env, path::Path};

use anyhow::{Context, Result, bail};
use log::{debug, info, log_enabled, warn};
use zenith_config::{CONFIG_FILE_NAME, Config, LoadError};
use zenith_contracts::Contract;
use zenith_diagnostics::{Diagnostic, ReportKind, SourceMap, message};
use zenith_pipeline::{FileError, FileErrorKind, LANGUAGES, Pipeline, Session, StageKind};
use zenith_utils::{
    fs::{FileSystem, disk},
    path::adjust_canonicalisation,
};
use zenith_workspace::Checkout;

use crate::{Reported, cli::GlobalArgs, reporting, version::version};

/// Open a session for the project that the global options point at.
///
/// The configuration file is `--config`, or `zenith.toml` in `--root`, or else
/// the nearest `zenith.toml` in or above the current directory. The root is
/// `--root`, or else the directory of the configuration file. A project
/// without a configuration file is rooted at the current directory, and uses
/// the defaults. With `--tracked`, only the files that git tracks are scanned.
/// The session keeps a parse cache, unless `--no-cache` or the configuration
/// turns it off.
pub(crate) fn open(global: &GlobalArgs) -> Result<Session> {
    // 1. Find the configuration file, if there is one.
    let file_system = disk().clone();
    let cwd = env::current_dir().with_context(|| message!(cli_current_dir))?;
    let config_path = match (&global.config, &global.root) {
        (Some(config), _) if !file_system.is_file(config) => {
            bail!(message!(cli_config_not_found, path = config.display()));
        }
        (Some(config), _) => Some(config.clone()),
        (None, Some(root)) => {
            Some(root.join(CONFIG_FILE_NAME)).filter(|path| file_system.is_file(path))
        }
        (None, None) => Config::discover(&*file_system, &cwd),
    };

    // 2. Find the project root.
    let root = match (&global.root, &config_path) {
        (Some(root), _) => root.as_path(),
        // A bare file name, e.g. `--config zenith.toml`, has an empty parent.
        (None, Some(config)) => {
            config.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(&cwd)
        }
        (None, None) => &cwd,
    };
    if !file_system.is_dir(root) {
        bail!(message!(cli_root_not_a_directory, root = root.display()));
    }
    let root = file_system.adjust_canonicalisation(root);
    debug!("project root: {}", root.display());

    // 3. Load the configuration, or use the defaults.
    let config = match &config_path {
        Some(path) => {
            debug!("configuration: {}", path.display());
            load(&*file_system, path, &root)?
        }
        None => {
            debug!("no `{CONFIG_FILE_NAME}` found; using the defaults");
            Config::defaults(LANGUAGES)
        }
    };

    // 4. Keep a parse cache for this version of zenith, unless it is turned off.
    let mut session = Session::with_file_system(root, file_system, config);
    session.tracked_only = global.tracked;
    if !global.no_cache {
        session.cache = session.cache_settings(&version().to_string());
    }
    Ok(session)
}

/// Build the graph of the project of `project` as it is in `checkout`, the
/// commit that `reference` names: rooted at the same path in the checkout,
/// and configured by the configuration file at the same path, or by the one
/// in that root, or else by the defaults. A configuration file outside the
/// repository configures it as it is.
///
/// Every file of the commit is scanned, without a parse cache, and nothing is
/// said of them: what is wrong with another commit's files is not the working
/// tree's to fix. The errors name the reference, not the checkout's temporary
/// directory.
pub(crate) fn build_at(
    global: &GlobalArgs,
    project: &Session,
    checkout: &Checkout,
    reference: &str,
) -> Result<Session> {
    // 1. Find the root and the configuration file at the same paths in the
    //    checkout.
    let root = checkout.path_of(&project.root);
    if !root.is_dir() {
        bail!(message!(cli_graph_diff_no_root, reference));
    }
    let config = project
        .config
        .file
        .as_ref()
        .map(|file| checkout.path_of(&adjust_canonicalisation(&file.path)));
    let global = GlobalArgs {
        config: config.filter(|path| path.is_file()),
        root: Some(root),
        tracked: false,
        no_cache: true,
        ..global.clone()
    };

    // 2. Build the graph, without reporting on the files.
    let failed = || message!(cli_graph_diff_failed, reference);
    let mut session = open(&global).with_context(failed)?;
    Pipeline::standard().run_until(&mut session, StageKind::Graph).with_context(failed)?;
    session.narrow();
    if session.files.is_empty() {
        bail!(message!(cli_graph_diff_no_source_files, reference));
    }
    Ok(session)
}

/// Run the standard pipeline on `session`, up to and including the `last`
/// stage, keep only the problems of the files in its scope, and warn about
/// those that could not be read or parsed. Returns
/// the pipeline, which knows how long each stage took.
///
/// A graph without source files is an error: the root is most likely wrong,
/// and every path selector would fail to match.
pub(crate) fn run_until(session: &mut Session, last: StageKind) -> Result<Pipeline> {
    let mut pipeline = Pipeline::standard();
    pipeline.run_until(session, last)?;
    session.narrow();
    report_file_errors(&mut SourceMap::with_root(&session.root), &session.file_errors);

    if last >= StageKind::Graph {
        if session.files.is_empty() {
            bail!(message!(cli_no_source_files, root = session.root.display()));
        }
        let (files, imports) = (session.files.len(), session.graph.detail_count());
        info!("{}", message!(cli_indexed, files, imports));
        if session.cache.is_some() {
            info!("{}", message!(cli_reused_from_cache, files = session.stats.cached_files));
        }
        match session.stats.non_literal_imports {
            0 => {}
            imports => info!("{}", message!(cli_non_literal_imports, imports)),
        }
    }
    Ok(pipeline)
}

/// Compile the contracts of the session's configuration whose ids are in
/// `ids`, or every contract if there are none, against its graph. An unknown
/// id is an error. So are the contracts' errors, which are shown in the
/// context of the configuration file.
pub(crate) fn compile_contracts<'s>(
    session: &'s Session,
    ids: &[String],
) -> Result<Vec<Contract<'s>>> {
    let configs = &session.config.contracts;
    for id in ids {
        if session.config.contract(id).is_none() {
            let known: Vec<_> = configs.iter().map(|config| format!("`{}`", config.id)).collect();
            match known.as_slice() {
                [] => bail!(message!(cli_unknown_contract_without_contracts, id)),
                known => bail!(message!(cli_unknown_contract, id, contracts = known.join(", "))),
            }
        }
    }
    let selected = configs.iter().filter(|config| ids.is_empty() || ids.contains(&config.id));
    Contract::compile_all(selected, &session.graph, &session.workspace).map_err(|errors| {
        let (sources, config) = session.sources();
        let reports: Vec<_> =
            errors.iter().map(|error| error.report_in(ReportKind::Error, config)).collect();
        reporting::emit(&sources, &reports);
        Reported.into()
    })
}

/// Load the configuration file at `path` in `file_system`. An invalid file is
/// reported, with its path relative to the project `root` if it is inside it.
fn load(file_system: &dyn FileSystem, path: &Path, root: &Path) -> Result<Config> {
    match Config::load(file_system, path, LANGUAGES) {
        Ok(config) => Ok(config),
        Err(LoadError::Invalid { path, contents, errors }) => {
            let mut sources = SourceMap::with_root(root);
            let source = sources.add_file(&path, contents);
            let reports: Vec<_> = errors
                .iter()
                .map(|error| error.report_in(ReportKind::Error, Some(source)))
                .collect();
            reporting::emit(&sources, &reports);
            Err(Reported.into())
        }
        Err(error) => Err(error.into()),
    }
}

/// Warn about the files that could not be read or parsed in full. A syntax
/// error is shown in context.
fn report_file_errors(sources: &mut SourceMap, errors: &[FileError]) {
    let reports: Vec<_> = errors
        .iter()
        .map(|error| {
            // Only a syntax error is shown in context, so only its file is read.
            let source = match error.kind {
                FileErrorKind::Syntax(_) => sources.file(&error.file),
                FileErrorKind::Read(_) => None,
            };
            error.report(source)
        })
        .collect();
    reporting::emit(sources, &reports);
}

/// Warn about the imports of `session` that could not be resolved, and list
/// them with `-v`.
pub(crate) fn warn_about_unresolved(session: &Session) {
    let imports = session.unresolved.len();
    if imports == 0 {
        return;
    }
    let mut sources = SourceMap::with_root(&session.root);
    let listed = log_enabled!(log::Level::Info);
    if listed {
        for import in &session.unresolved {
            let unresolved = message!(
                cli_unresolved_import,
                specifier = import.import.specifier,
                location = import.location(&mut sources),
                reason = import.reason
            );
            info!("{unresolved}");
        }
    }
    match listed {
        true => warn!("{}", message!(cli_unresolved, imports)),
        false => warn!("{}", message!(cli_unresolved_unlisted, imports)),
    }
}
