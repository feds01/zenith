//! `zenith init`.

use std::{env, fs};

use anyhow::{Context, Result, bail};
use zenith_config::{CONFIG_FILE_NAME, Config};
use zenith_contracts::Suggestion;
use zenith_diagnostics::message;
use zenith_pipeline::{LANGUAGES, Session, StageKind};
use zenith_utils::fs::adjust_canonicalisation;

use crate::{
    ExitStatus,
    cli::{GlobalArgs, InitCommand},
    project,
};

/// The starter configuration: a `#:schema` line that points editors at the
/// JSON Schema, then a contract that keeps the workspace packages free of
/// cycles.
const STARTER: &str = include_str!("starter.toml");

/// Write a starter `zenith.toml` from the discovered packages, with
/// `--suggest` followed by the contracts that the code already keeps.
pub fn init(global: &GlobalArgs, args: InitCommand) -> Result<ExitStatus> {
    // 1. Find where to write the configuration: `--config`, or else `zenith.toml`
    //    in `--root` or the current directory. An existing file is only overwritten
    //    with `--force`.
    let cwd = env::current_dir().with_context(|| message!(cli_current_dir))?;
    let root = global.root.as_deref().unwrap_or(&cwd);
    if !root.is_dir() {
        bail!(message!(cli_root_not_a_directory, root = root.display()));
    }
    let root = adjust_canonicalisation(root);
    let path = global.config.clone().unwrap_or_else(|| root.join(CONFIG_FILE_NAME));
    let shown = path.strip_prefix(adjust_canonicalisation(&cwd)).unwrap_or(&path).display();
    if path.exists() && !args.force {
        bail!(message!(cli_init_exists, path = shown));
    }

    // 2. Find the source files and workspace packages, as the default configuration
    //    does, and with `--suggest`, build their graph, as `zenith suggest` does.
    let mut session = Session::new(root, Config::defaults(LANGUAGES));
    session.tracked_only = global.tracked;
    if args.suggest {
        project::build_graph_for_suggestions(&mut session)?;
    } else {
        project::run_until(&mut session, StageKind::Discover)?;
    }

    // 3. Start from the starter configuration: the workspace packages, if there are
    //    two or more, must not depend on each other in cycles.
    let packages = &session.workspace.packages;
    let starter = match packages.len() {
        0 | 1 => STARTER.split_inclusive('\n').next().unwrap_or_default(),
        _ => STARTER,
    };

    // 4. With `--suggest`, add the contracts that the code keeps, but that the
    //    starter's doesn't cover.
    let mut contents = starter.to_string();
    let suggestions = match args.suggest {
        true => {
            let config = Config::parse(starter, LANGUAGES).expect("the starter is valid");
            Suggestion::find(project::graphs(&session), &session.workspace, &config, LANGUAGES)
        }
        false => Vec::new(),
    };
    for suggestion in &suggestions {
        contents.push('\n');
        contents.push_str(&suggestion.to_string());
    }

    // 5. Write it.
    fs::write(&path, contents).with_context(|| message!(cli_init_write_failed, path = shown))?;
    let written = message!(
        cli_init_written,
        path = shown,
        files = session.files.len(),
        packages = packages.len()
    );
    println!("{written}");
    if args.suggest {
        println!("{}", message!(cli_init_suggested, contracts = suggestions.len()));
    }
    Ok(ExitStatus::Success)
}
