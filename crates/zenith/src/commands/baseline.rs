//! `zenith baseline`.

use std::{fs, io};

use anyhow::{Context, Result};
use log::warn;
use zenith_contracts::{Baseline, Exemptions};
use zenith_pipeline::StageKind;
use zenith_utils::printing::{plural, thousands};
use zenith_workspace::Git;

use crate::{
    ExitStatus,
    cli::{BaselineCommand, GlobalArgs},
    project,
};

/// Record the current violations in the baseline file, so that `zenith
/// check` only fails on new ones, or only remove the stale entries with
/// `--prune`.
pub fn baseline(global: &GlobalArgs, args: BaselineCommand) -> Result<ExitStatus> {
    // 1. Open the project, and build its graph.
    let mut session = project::open(global)?;
    project::run_until(&mut session, StageKind::Graph)?;

    // 2. Compile the contracts to record: every one, or those of `--contract`.
    let contracts = project::compile_contracts(&session, &args.contracts)?;

    // 3. Read the baseline, if there is one.
    let path = session.baseline_path();
    let old = Baseline::load(&path)?.unwrap_or_default();
    let mut baseline = old.clone();

    // 4. Find the entries of the contracts afresh, or only remove the stale ones,
    //    after following the files that git sees renamed, so that their entries
    //    move with them, and aren't counted as new. The entries of other contracts
    //    are kept, unless, without `--contract`, the contracts no longer exist.
    let mut known = old.clone();
    if args.prune {
        let ids: Vec<_> = contracts.iter().map(|contract| contract.config.id.as_str()).collect();
        known.follow_renames(ids.iter().copied(), &Git::new(&session.root).renames());
        baseline = known.clone();
        baseline.prune(ids, &session.graph);
    } else {
        let exemptions = Exemptions { suppressions: &session.suppressions, baseline: None };
        baseline.record(&contracts, &session.graph, &session.workspace, &exemptions);
    }
    if args.contracts.is_empty() {
        baseline.retain(|id| session.config.contract(id).is_some());
    }

    // 5. Write the baseline, or remove it if it has no entries.
    let shown = &session.config.baseline.path;
    if baseline.is_empty() {
        match fs::remove_file(&path) {
            Ok(()) => println!("No violations remain; removed `{shown}`."),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                println!("No violations to record.");
            }
            Err(error) => {
                return Err(error).with_context(|| format!("could not remove `{shown}`"));
            }
        }
        return Ok(ExitStatus::Success);
    }
    baseline.save(&path)?;
    let after = baseline.len();
    println!(
        "Wrote {} {} for {} to `{shown}` (was {}).",
        thousands(after),
        if after == 1 { "baseline entry" } else { "baseline entries" },
        plural(baseline.contracts.len(), "contract"),
        thousands(old.len())
    );
    let new = baseline.count_new(&known);
    if new > 0 {
        let them = if new == 1 { "it" } else { "them" };
        warn!("the baseline gained {}; review {them} before committing", plural(new, "entry"));
    }
    Ok(ExitStatus::Success)
}
