//! `zenith resolve`: how one specifier resolves from a file.

use anyhow::{Result, bail};
use zenith_resolve::{Resolved, Resolver};
use zenith_utils::path::{adjust_canonicalisation, relative_posix};
use zenith_workspace::{discover_packages, walk};

use crate::{
    ExitStatus,
    cli::{GlobalArgs, ResolveCommand},
    project::Project,
};

pub fn resolve(global: &GlobalArgs, args: ResolveCommand) -> Result<ExitStatus> {
    let project = Project::locate(global, false)?;
    let file = adjust_canonicalisation(&args.file);
    if !file.is_file() {
        bail!("`{}` is not a file", args.file.display());
    }

    let settings = project.index_settings()?;
    let files = walk(&project.root, &settings.walk);
    let workspace = discover_packages(&project.root, &files.package_jsons);
    let mut resolver_settings = settings.resolver;
    resolver_settings.workspace_packages = workspace
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.path.clone()))
        .collect();
    let resolver = Resolver::new(&resolver_settings);

    match resolver.resolve(&file, &args.specifier) {
        Resolved::Internal(path) => {
            let relative =
                relative_posix(&project.root, &path).unwrap_or_else(|| path.display().to_string());
            println!("internal  {relative}");
        }
        Resolved::External { package, unresolved } => {
            let note =
                if unresolved { "  (not installed; classified from the specifier)" } else { "" };
            println!("external  npm:{package}{note}");
        }
        Resolved::Builtin(name) => println!("builtin   {name}"),
        Resolved::Unresolved(reason) => {
            println!("unresolved  {reason}");
            return Ok(ExitStatus::Failure);
        }
    }
    Ok(ExitStatus::Success)
}
