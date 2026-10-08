//! The contract types, and compiling them from configuration.

mod acyclic;
mod encapsulation;
mod forbidden;
mod independence;
mod layers;
mod protected;

use zenith_config::{AlertLevel, Config, ContractConfig, SelectorConfig, Severity};
use zenith_graph::{GraphView, KindSet, NodeSet};
use zenith_index::Index;

use crate::{
    findings::Violation,
    selector::{Selector, Unit},
    view::IgnoreImport,
};

/// A compiled contract, with its selectors resolved against the project.
#[derive(Debug, Clone)]
pub struct Contract {
    pub id: String,
    pub name: String,
    pub severity: Severity,
    pub guidance: Option<String>,
    pub exclude_kinds: KindSet,
    pub ignore_imports: Vec<IgnoreImport>,
    pub unmatched_ignore_imports: AlertLevel,
    kind: ContractKind,
}

#[derive(Debug, Clone)]
enum ContractKind {
    Layers(layers::Layers),
    Forbidden(forbidden::Forbidden),
    Independence(independence::Independence),
    Protected(protected::Protected),
    Acyclic(acyclic::Acyclic),
    Encapsulation(encapsulation::Encapsulation),
}

impl Contract {
    /// The contract type, as written in `type`.
    pub fn type_name(&self) -> &'static str {
        match &self.kind {
            ContractKind::Layers(_) => "layers",
            ContractKind::Forbidden(_) => "forbidden",
            ContractKind::Independence(_) => "independence",
            ContractKind::Protected(_) => "protected",
            ContractKind::Acyclic(_) => "acyclic",
            ContractKind::Encapsulation(_) => "encapsulation",
        }
    }

    /// Check the contract against a view of the graph.
    pub(crate) fn check(&self, view: &GraphView) -> Vec<Violation> {
        let kinds = self.exclude_kinds;
        match &self.kind {
            ContractKind::Layers(contract) => contract.check(view, kinds),
            ContractKind::Forbidden(contract) => contract.check(view, kinds),
            ContractKind::Independence(contract) => contract.check(view, kinds),
            ContractKind::Protected(contract) => contract.check(view, kinds),
            ContractKind::Acyclic(contract) => contract.check(view, kinds),
            ContractKind::Encapsulation(contract) => contract.check(view, kinds),
        }
    }
}

/// Compile every contract in `config` against the project. Errors are
/// collected for all contracts, each prefixed with the contract id.
pub fn compile(config: &Config, index: &Index) -> Result<Vec<Contract>, Vec<String>> {
    let mut contracts = Vec::new();
    let mut errors = Vec::new();

    for (id, contract) in &config.contracts {
        match compile_one(config, index, id, contract) {
            Ok(contract) => contracts.push(contract),
            Err(error) => errors.push(format!("contract `{id}`: {error}")),
        }
    }

    if errors.is_empty() { Ok(contracts) } else { Err(errors) }
}

fn compile_one(
    config: &Config,
    index: &Index,
    id: &str,
    contract: &ContractConfig,
) -> Result<Contract, String> {
    let common = contract.common();
    let exclude_kinds = common
        .exclude_kinds
        .as_ref()
        .unwrap_or(&config.analysis.exclude_kinds)
        .iter()
        .copied()
        .collect();
    let ignore_imports = common
        .ignore_imports
        .iter()
        .map(|raw| IgnoreImport::parse(raw))
        .collect::<Result<_, _>>()?;

    let kind = match contract {
        ContractConfig::Layers(c) => ContractKind::Layers(layers::Layers::compile(index, c)?),
        ContractConfig::Forbidden(c) => {
            ContractKind::Forbidden(forbidden::Forbidden::compile(index, c)?)
        }
        ContractConfig::Independence(c) => {
            ContractKind::Independence(independence::Independence::compile(index, c)?)
        }
        ContractConfig::Protected(c) => {
            ContractKind::Protected(protected::Protected::compile(index, c)?)
        }
        ContractConfig::Acyclic(c) => ContractKind::Acyclic(acyclic::Acyclic::compile(index, c)?),
        ContractConfig::Encapsulation(c) => {
            ContractKind::Encapsulation(encapsulation::Encapsulation::compile(index, c)?)
        }
    };

    Ok(Contract {
        id: id.to_string(),
        name: common.name.clone().unwrap_or_else(|| id.to_string()),
        severity: common.severity,
        guidance: common.guidance.clone(),
        exclude_kinds,
        ignore_imports,
        unmatched_ignore_imports: common.unmatched_ignore_imports,
        kind,
    })
}

/// Resolve selectors from configuration into units, failing if a selector
/// that must match something matches nothing.
pub(crate) fn resolve_units(
    index: &Index,
    field: &str,
    selectors: &[SelectorConfig],
) -> Result<Vec<Unit>, String> {
    let mut units = Vec::new();
    for config in selectors {
        let selector = Selector::from_config(config)?;
        let (resolved, base_matched) = selector.resolve_with_base(index);
        if !base_matched && selector.must_match() {
            return Err(format!(
                "`{field}` selector `{}` does not match anything",
                selector.as_str()
            ));
        }
        for unit in resolved {
            if !units
                .iter()
                .any(|existing: &Unit| existing.root.is_some() && existing.root == unit.root)
            {
                units.push(unit);
            }
        }
    }
    Ok(units)
}

/// Fail if any two units share members.
pub(crate) fn ensure_disjoint(field: &str, units: &[Unit]) -> Result<(), String> {
    for (index, a) in units.iter().enumerate() {
        for b in &units[index + 1..] {
            if a.members.intersects(&b.members) {
                return Err(format!(
                    "`{field}` contains overlapping modules `{}` and `{}`",
                    a.label, b.label
                ));
            }
        }
    }
    Ok(())
}

/// The members of several units, combined.
pub(crate) fn all_members(units: &[Unit]) -> NodeSet {
    crate::selector::union_members(units)
}
