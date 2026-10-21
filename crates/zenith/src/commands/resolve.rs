//! `zenith resolve`.

use anyhow::{Result, bail};
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
        bail!("`{}` is not a file", args.file.display());
    }

    // 2. Find the file's language, and the workspace packages, which resolve
    //    without `node_modules`.
    let language = relative_posix(&session.root, &file)
        .and_then(|name| session.config.languages.language_of(&name));
    let Some(language) = language else {
        bail!("`{}` is not a source file of an enabled language", args.file.display());
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
    let (kind, target) = match resolved {
        Resolved::Internal(path) => ("internal", path),
        Resolved::Package(dir) => ("package", dir),
        Resolved::External { package, found: true } => ("external", format!("{scheme}:{package}")),
        Resolved::External { package, found: false } => {
            ("external", format!("{scheme}:{package} (not installed, so named from the specifier)"))
        }
        Resolved::Builtin(name) => ("builtin", name),
        Resolved::Unresolved(reason) => ("unresolved", reason.to_string()),
        Resolved::Ignored(reason) => {
            ("unresolved", format!("{reason} (expected, by `ignore_unresolved`)"))
        }
    };
    println!("{kind}: {target}");
    Ok(status)
}
