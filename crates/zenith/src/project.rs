//! Finding the project and its configuration, and running the pipeline on
//! it.

use std::{env, path::Path};

use anyhow::{Context, Result, bail};
use log::{debug, info, log_enabled, warn};
use zenith_config::{CONFIG_FILE_NAME, Config, LoadError};
use zenith_contracts::ContractError;
use zenith_diagnostics::SourceMap;
use zenith_pipeline::{FileError, FileErrorKind, Pipeline, Session, StageKind};
use zenith_reporting::Sources;
use zenith_utils::{path::adjust_canonicalisation, printing::plural};

use crate::{Reported, cli::GlobalArgs, reporting};

/// Open a session for the project that the global options point at.
///
/// The configuration file is `--config`, or `zenith.toml` in `--root`, or else
/// the nearest `zenith.toml` in or above the current directory. The root is
/// `--root`, or else the directory of the configuration file. A project
/// without a configuration file is rooted at the current directory, and uses
/// the defaults.
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
            load(path, &cwd)?
        }
        None => {
            debug!("no `{CONFIG_FILE_NAME}` found; using the defaults");
            Config::default()
        }
    };
    Ok(Session::new(root, config))
}

/// Run the standard pipeline on `session`, up to and including the `last`
/// stage, and warn about the files that could not be read or parsed. Returns
/// the pipeline, which knows how long each stage took.
pub(crate) fn run_until(session: &mut Session, last: StageKind) -> Result<Pipeline> {
    let mut pipeline = Pipeline::standard();
    pipeline.run_until(session, last)?;
    report_file_errors(&mut Sources::new(&session.root), &session.file_errors);

    if last >= StageKind::Graph {
        info!(
            "indexed {} and {}",
            plural(session.files.len(), "source file"),
            plural(session.graph.detail_count(), "import")
        );
        match session.stats.non_literal_imports {
            0 => {}
            1 => info!("skipped 1 import whose specifier is not a literal"),
            count => info!("skipped {count} imports whose specifier is not a literal"),
        }
    }
    Ok(pipeline)
}

/// Show the errors of the contracts that could not be compiled, in the
/// context of the configuration file.
pub(crate) fn report_contract_errors(session: &Session, errors: &[ContractError]) {
    let mut sources = SourceMap::new();
    let source = session.config.file.as_ref().map(|file| {
        let cwd = env::current_dir().unwrap_or_default();
        let shown = file.path.strip_prefix(&cwd).unwrap_or(&file.path);
        sources.add(shown.display().to_string(), file.contents.as_str())
    });
    let reports: Vec<_> = errors
        .iter()
        .map(|error| match source {
            Some(source) => error.report(source),
            // Without a file, there is no excerpt to show.
            None => zenith_diagnostics::Report::error(error.to_string()),
        })
        .collect();
    reporting::emit(&sources, &reports);
}

/// Load the configuration file at `path`. An invalid file is reported, with
/// its path relative to `cwd` if it is inside it.
fn load(path: &Path, cwd: &Path) -> Result<Config> {
    match Config::load(path) {
        Ok(config) => Ok(config),
        Err(LoadError::Invalid { path, contents, errors }) => {
            let mut sources = SourceMap::new();
            let shown = path.strip_prefix(cwd).unwrap_or(&path);
            let source = sources.add(shown.display().to_string(), contents);
            let reports: Vec<_> = errors.iter().map(|error| error.report(source)).collect();
            reporting::emit(&sources, &reports);
            Err(Reported.into())
        }
        Err(error) => Err(error.into()),
    }
}

/// Warn about the files that could not be read or parsed in full. A syntax
/// error is shown in context.
fn report_file_errors(sources: &mut Sources, errors: &[FileError]) {
    let reports: Vec<_> = errors
        .iter()
        .map(|error| {
            // Only a syntax error is shown in context, so only its file is read.
            let source = match error.kind {
                FileErrorKind::Syntax(_) => sources.source(&error.file),
                FileErrorKind::Read(_) => None,
            };
            error.report(source)
        })
        .collect();
    reporting::emit(sources.map(), &reports);
}

/// Warn about the imports of `session` that could not be resolved, and list
/// them with `-v`.
pub(crate) fn warn_about_unresolved(session: &Session) {
    let count = session.unresolved.len();
    if count == 0 {
        return;
    }
    let mut sources = Sources::new(&session.root);
    let listed = log_enabled!(log::Level::Info);
    if listed {
        for import in &session.unresolved {
            let (line, column) =
                sources.position(&import.file, import.import.range.start()).unwrap_or_default();
            info!(
                "`{}` in {}:{line}:{column} could not be resolved: {}",
                import.import.specifier, import.file, import.reason
            );
        }
    }
    let hint = if listed { "" } else { "; run with `-v` to list them" };
    warn!("{} could not be resolved{hint}", plural(count, "import"));
}
