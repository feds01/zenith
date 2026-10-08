//! `zenith baseline`: record the current violations, so that only new ones
//! fail `zenith check`.

use anyhow::Result;
use log::warn;
use zenith_contracts::{Baseline, generate_baseline};
use zenith_utils::{pluralise, printing::thousands};

use crate::{
    ExitStatus,
    cli::{BaselineCommand, GlobalArgs},
    commands::compile_contracts,
    project::Project,
};

pub fn baseline(global: &GlobalArgs, args: BaselineCommand) -> Result<ExitStatus> {
    let project = Project::locate(global, true)?;
    let index = project.index()?;
    let contracts = compile_contracts(&project, &index, &args.contracts)?;

    let path = project.baseline_path();
    let mut baseline =
        Baseline::load(&path).map_err(anyhow::Error::msg)?.unwrap_or_else(Baseline::new);
    let before: usize = baseline.contracts.values().map(Vec::len).sum();

    // Forget contracts that no longer exist, unless only some were selected.
    if args.contracts.is_empty() {
        baseline.contracts.retain(|id, _| project.config.contracts.contains_key(id));
    }

    let graph = &index.graph;
    for contract in &contracts {
        let entries = if args.prune {
            baseline
                .entries(&contract.id)
                .iter()
                .filter(|entry| {
                    entry.split_once(" -> ").is_some_and(|(importer, imported)| {
                        match (graph.id(importer), graph.id(imported)) {
                            (Some(importer), Some(imported)) => {
                                graph.has_import(importer, imported)
                            }
                            _ => false,
                        }
                    })
                })
                .cloned()
                .collect()
        } else {
            generate_baseline(&index, contract)
        };
        if entries.is_empty() {
            baseline.contracts.remove(&contract.id);
        } else {
            baseline.contracts.insert(contract.id.clone(), entries);
        }
    }

    let after: usize = baseline.contracts.values().map(Vec::len).sum();
    if after == 0 {
        if path.exists() {
            std::fs::remove_file(&path)?;
            println!("No violations remain; removed `{}`.", project.config.baseline.path);
        } else {
            println!("No violations to record.");
        }
        return Ok(ExitStatus::Success);
    }

    baseline.save(&path).map_err(anyhow::Error::msg)?;
    let contracts_count = baseline.contracts.len();
    println!(
        "Wrote {} baseline entr{} for {contracts_count} contract{} to `{}` (was {}).",
        thousands(after),
        if after == 1 { "y" } else { "ies" },
        pluralise!(contracts_count),
        project.config.baseline.path,
        thousands(before)
    );
    if !args.prune && after > before {
        warn!(
            "the baseline grew by {}; review the new entries before committing them",
            after - before
        );
    }
    Ok(ExitStatus::Success)
}
