//! Finding the project and its configuration, and running the pipeline on
//! it.

use std::{env, path::Path};

use anyhow::{Context, Result, bail};
use log::{debug, info, log_enabled, warn};
use zenith_config::{CONFIG_FILE_NAME, Config, LoadError};
use zenith_contracts::Contract;
use zenith_diagnostics::SourceMap;
use zenith_pipeline::{FileError, FileErrorKind, Pipeline, Session, StageKind};
use zenith_utils::{path::adjust_canonicalisation, printing::plural};

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
    let cwd = env::current_dir().context("could not read the current directory")?;
    let config_path = match (&global.config, &global.root) {
        (Some(config), _) if !config.is_file() => {
            bail!("the configuration file `{}` does not exist", config.display());
        }
        (Some(config), _) => Some(config.clone()),
        (None, Some(root)) => Some(root.join(CONFIG_FILE_NAME)).filter(|path| path.is_file()),
        (None, None) => Config::discover(&cwd),
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
    if !root.is_dir() {
        bail!("the project root `{}` is not a directory", root.display());
    }
    let root = adjust_canonicalisation(root);
    debug!("project root: {}", root.display());

    // 3. Load the configuration, or use the defaults.
    let config = match &config_path {
        Some(path) => {
            debug!("configuration: {}", path.display());
            load(path, &root)?
        }
        None => {
            debug!("no `{CONFIG_FILE_NAME}` found; using the defaults");
            Config::default()
        }
    };

    // 4. Keep a parse cache for this version of zenith, unless it is turned off.
    let mut session = Session::new(root, config);
    session.tracked_only = global.tracked;
    if !global.no_cache {
        session.cache = session.cache_settings(&version().to_string());
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
            bail!(
                "found no source files in the project root `{}`; use `--root` to point at the project",
                session.root.display()
            );
        }
        info!(
            "indexed {} and {}",
            plural(session.files.len(), "source file"),
            plural(session.graph.detail_count(), "import")
        );
        if session.cache.is_some() {
            info!(
                "reused the imports of {} of them from the parse cache",
                session.stats.cached_files
            );
        }
        match session.stats.non_literal_imports {
            0 => {}
            1 => info!("skipped 1 import whose specifier is not a literal"),
            count => info!("skipped {count} imports whose specifier is not a literal"),
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
                [] => bail!("there is no contract `{id}`; the configuration has no contracts"),
                known => {
                    bail!("there is no contract `{id}`; the contracts are {}", known.join(", "))
                }
            }
        }
    }
    let selected = configs.iter().filter(|config| ids.is_empty() || ids.contains(&config.id));
    Contract::compile_all(selected, &session.graph, &session.workspace).map_err(|errors| {
        let (sources, config) = session.sources();
        let reports: Vec<_> = errors.iter().map(|error| error.report(config)).collect();
        reporting::emit(&sources, &reports);
        Reported.into()
    })
}

/// Load the configuration file at `path`. An invalid file is reported, with
/// its path relative to the project `root` if it is inside it.
fn load(path: &Path, root: &Path) -> Result<Config> {
    match Config::load(path) {
        Ok(config) => Ok(config),
        Err(LoadError::Invalid { path, contents, errors }) => {
            let mut sources = SourceMap::with_root(root);
            let source = sources.add_file(&path, contents);
            let reports: Vec<_> = errors.iter().map(|error| error.report(source)).collect();
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
    let count = session.unresolved.len();
    if count == 0 {
        return;
    }
    let mut sources = SourceMap::with_root(&session.root);
    let listed = log_enabled!(log::Level::Info);
    if listed {
        for import in &session.unresolved {
            info!(
                "`{}` in {} could not be resolved: {}",
                import.import.specifier,
                import.location(&mut sources),
                import.reason
            );
        }
    }
    let hint = if listed { "" } else { "; run with `-v` to list them" };
    warn!("{} could not be resolved{hint}", plural(count, "import"));
}
