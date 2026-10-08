//! The `zenith` subcommands.

pub mod baseline;
pub mod check;
pub mod graph;
pub mod init;
pub mod resolve;
pub mod schema;
pub mod version;
pub mod why;

use anyhow::{Result, bail};
use log::error;
use zenith_contracts::Contract;
use zenith_index::Index;

use crate::project::Project;

/// Compile the project's contracts, keeping only `selected` ids if any are
/// given. Configuration errors are logged and turned into a single error.
pub(crate) fn compile_contracts(
    project: &Project,
    index: &Index,
    selected: &[String],
) -> Result<Vec<Contract>> {
    for id in selected {
        if !project.config.contracts.contains_key(id) {
            let known: Vec<_> =
                project.config.contracts.keys().map(|id| format!("`{id}`")).collect();
            bail!("there is no contract `{id}`; the contracts are {}", known.join(", "));
        }
    }

    let contracts = match zenith_contracts::compile(&project.config, index) {
        Ok(contracts) => contracts,
        Err(errors) => {
            for message in &errors {
                error!("{message}");
            }
            let count = errors.len();
            bail!("{count} contract{} could not be compiled", if count == 1 { "" } else { "s" });
        }
    };

    Ok(contracts
        .into_iter()
        .filter(|contract| selected.is_empty() || selected.contains(&contract.id))
        .collect())
}
