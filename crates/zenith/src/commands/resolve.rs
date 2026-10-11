//! `zenith resolve`.

use anyhow::{Result, bail};
use zenith_pipeline::StageKind;
use zenith_resolve::Resolved;
use zenith_utils::path::adjust_canonicalisation;

use crate::{
    ExitStatus,
    cli::{GlobalArgs, ResolveCommand},
    project,
};

/// Show how one import specifier resolves from a file, and why. Exits with
/// [`ExitStatus::Failure`] when it doesn't resolve.
pub fn resolve(global: &GlobalArgs, args: ResolveCommand) -> Result<ExitStatus> {
    let mut session = project::open(global)?;
    let file = adjust_canonicalisation(&args.file);
    if !file.is_file() {
        bail!("`{}` is not a file", args.file.display());
    }

    // Workspace packages resolve without `node_modules`, so find them first.
    project::run_until(&mut session, StageKind::Discover)?;
    let resolver = session.resolver()?;

    let resolved = resolver.resolve(&file, &args.specifier);
    let status = match resolved {
        Resolved::Unresolved(_) | Resolved::Ignored(_) => ExitStatus::Failure,
        _ => ExitStatus::Success,
    };
    let (kind, target) = match resolved {
        Resolved::Internal(path) => ("internal", path),
        Resolved::External { package, found: true } => ("external", format!("npm:{package}")),
        Resolved::External { package, found: false } => {
            ("external", format!("npm:{package} (not installed, so named from the specifier)"))
        }
        Resolved::Builtin(name) => ("builtin", name),
        Resolved::Unresolved(reason) => ("unresolved", reason.to_string()),
        Resolved::Ignored(reason) => {
            ("unresolved", format!("{reason} (expected, by `ignore_unresolved`)"))
        }
    };
    println!("{kind:<10}  {target}");
    Ok(status)
}
