//! Finding the project and its configuration, and running the pipeline on
//! it.

use std::{env, path::Path};

use anyhow::{Context, Result, bail};
use log::{debug, info, log_enabled, warn};
use zenith_config::{CONFIG_FILE_NAME, Config, LoadError};
use zenith_diagnostics::{ReportKind, SourceMap, Span};
use zenith_pipeline::{FileError, FileErrorKind, Pipeline, Session, StageKind};
use zenith_utils::{path::adjust_canonicalisation, printing::plural};

use crate::{Reported, cli::GlobalArgs, reporting, sources::SourceFiles};

/// Open a session for the project that the global options point at.
///
/// The configuration file is `--config`, or `zenith.toml` in `--root`, or else
/// the nearest `zenith.toml` in or above the current directory. The root is
/// `--root`, or else the directory of the configuration file. A project
/// without a configuration file is rooted at the current directory, and uses
/// the defaults.
pub(crate) fn open(global: &GlobalArgs) -> Result<Session> {
    let cwd = env::current_dir().context("could not read the current directory")?;
    let config_path = match (&global.config, &global.root) {
        (Some(config), _) if !config.is_file() => {
            bail!("the configuration file `{}` does not exist", config.display());
        }
        (Some(config), _) => Some(config.clone()),
        (None, Some(root)) => Some(root.join(CONFIG_FILE_NAME)).filter(|path| path.is_file()),
        (None, None) => Config::discover(&cwd),
    };

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
/// stage, and warn about the files and imports that are missing from the
/// graph.
pub(crate) fn run_until(session: &mut Session, last: StageKind) -> Result<()> {
    Pipeline::standard().run_until(session, last)?;

    let mut sources = SourceFiles::new(&session.root);
    for error in &session.file_errors {
        warn_about_file(&mut sources, error);
    }
    warn_about_unresolved(&mut sources, session);

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
    Ok(())
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

/// Warn about a file that could not be read or parsed in full. A syntax
/// error is shown in context.
fn warn_about_file(sources: &mut SourceFiles, error: &FileError) {
    let FileError { file, kind } = error;
    let errors = match kind {
        FileErrorKind::Read(error) => {
            warn!("could not read `{file}`, so its imports are missing: {error}");
            return;
        }
        FileErrorKind::Syntax(errors) => errors,
    };
    let (Some(first), Some(text)) = (errors.first(), sources.text(file)) else {
        warn!("`{file}` has syntax errors, so some of its imports may be missing");
        return;
    };

    let mut map = SourceMap::new();
    let source = map.add(file.as_str(), text);
    let mut report = first.value.report(Span::new(source, first.range));
    report.kind = ReportKind::Warning;
    report.add_note("some of the file's imports may be missing from the graph");
    if errors.len() > 1 {
        report.add_note(format!(
            "this is the first of {} in the file",
            plural(errors.len(), "syntax error")
        ));
    }
    reporting::emit(&map, &[report]);
}

/// Warn about the imports that could not be resolved, and list them with
/// `-v`.
fn warn_about_unresolved(sources: &mut SourceFiles, session: &Session) {
    let count = session.unresolved.len();
    if count == 0 {
        return;
    }
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
