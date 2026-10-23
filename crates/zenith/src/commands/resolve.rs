//! `zenith resolve`.

use anyhow::{Result, bail};
use zenith_diagnostics::message;
use zenith_lang::Resolved;
use zenith_pipeline::{PipelineError, StageKind};
use zenith_utils::path::{adjust_canonicalisation, relative_posix};

use crate::{
    ExitStatus,
    cli::{GlobalArgs, ResolveCommand},
    project,
};

/// Show how one import specifier resolves from a file, and why. Exits with
/// [`ExitStatus::Failure`] when it doesn't resolve.
pub fn resolve(global: &GlobalArgs, args: ResolveCommand) -> Result<ExitStatus> {
    // 1. Open the project, and check the importing file.
    let mut session = project::open(global)?;
    let file = adjust_canonicalisation(&args.file);
    if !file.is_file() {
        bail!(message!(cli_not_a_file, path = args.file.display()));
    }

    // 2. Find the file's language, and the workspace packages, which resolve
    //    without `node_modules`.
    let language = relative_posix(&session.root, &file)
        .and_then(|name| session.config.languages.language_of(&name));
    let Some(language) = language else {
        bail!(message!(cli_not_a_source_file, path = args.file.display()));
    };
    project::run_until(&mut session, StageKind::Discover)?;
    let backend = session.backends.get(language);
    let resolver = backend.resolver().map_err(PipelineError::from)?;
    let scheme = backend.external_scheme();

    // 3. Resolve the specifier, and show what it resolves to.
    let resolved = resolver.resolve(&file, &args.specifier);
    let status = match resolved {
        Resolved::Unresolved(_) | Resolved::Ignored(_) => ExitStatus::Failure,
        _ => ExitStatus::Success,
    };
    let shown = match resolved {
        Resolved::Internal(path) => message!(cli_resolved_internal, path),
        Resolved::Package(dir) => message!(cli_resolved_package, dir),
        Resolved::External { package, found: true } => {
            message!(cli_resolved_external, scheme, package)
        }
        Resolved::External { package, found: false } => {
            message!(cli_resolved_not_installed, scheme, package)
        }
        Resolved::Builtin(name) => message!(cli_resolved_builtin, name),
        Resolved::Unresolved(reason) => message!(cli_resolved_unresolved, reason),
        Resolved::Ignored(reason) => message!(cli_resolved_ignored, reason),
    };
    println!("{shown}");
    Ok(status)
}
