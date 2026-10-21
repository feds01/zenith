//! `zenith init`.

use std::{env, fs};

use anyhow::{Context, Result, bail};
use zenith_config::{CONFIG_FILE_NAME, Config};
use zenith_pipeline::{LANGUAGES, Session, StageKind};
use zenith_utils::{path::adjust_canonicalisation, printing::plural};

use crate::{
    ExitStatus,
    cli::{GlobalArgs, InitCommand},
    project,
};

/// The starter configuration: a `#:schema` line that points editors at the
/// JSON Schema, then a contract that keeps the workspace packages free of
/// cycles.
const STARTER: &str = include_str!("starter.toml");

/// Write a starter `zenith.toml` from the discovered packages.
pub fn init(global: &GlobalArgs, args: InitCommand) -> Result<ExitStatus> {
    // 1. Find where to write the configuration: `--config`, or else `zenith.toml`
    //    in `--root` or the current directory. An existing file is only overwritten
    //    with `--force`.
    let cwd = env::current_dir().context("could not read the current directory")?;
    let root = global.root.as_deref().unwrap_or(&cwd);
    if !root.is_dir() {
        bail!("the project root `{}` is not a directory", root.display());
    }
    let root = adjust_canonicalisation(root);
    let path = global.config.clone().unwrap_or_else(|| root.join(CONFIG_FILE_NAME));
    let shown = path.strip_prefix(adjust_canonicalisation(&cwd)).unwrap_or(&path).display();
    if path.exists() && !args.force {
        bail!("`{shown}` already exists; use `--force` to overwrite it");
    }

    // 2. Find the source files and workspace packages, as the default configuration
    //    does.
    let mut session = Session::new(root, Config::defaults(LANGUAGES));
    session.tracked_only = global.tracked;
    project::run_until(&mut session, StageKind::Discover)?;

    // 3. Write the starter configuration: the workspace packages, if there are two
    //    or more, must not depend on each other in cycles.
    let packages = &session.workspace.packages;
    let starter = match packages.len() {
        0 | 1 => STARTER.split_inclusive('\n').next().unwrap_or_default(),
        _ => STARTER,
    };
    fs::write(&path, starter).with_context(|| format!("could not write `{shown}`"))?;
    println!(
        "Wrote `{shown}` ({} and {} found).",
        plural(session.files.len(), "source file"),
        plural(packages.len(), "workspace package")
    );
    Ok(ExitStatus::Success)
}
