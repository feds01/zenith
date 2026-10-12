//! The contract types: what each one checks, with its units resolved.

mod acyclic;
mod encapsulation;
mod forbidden;
mod independence;
mod layers;
mod protected;

use zenith_config::ContractKind;
use zenith_graph::{GraphView, KindSet};

use crate::{Violation, contract::UnitResolver};

/// What a contract checks: one variant per contract type, with the units
/// that its selectors resolved to.
#[derive(Debug)]
pub(crate) enum Rule {
    /// The modules must not form dependency cycles.
    Acyclic(acyclic::Acyclic),

    /// The modules may only be imported through their entry points.
    Encapsulation(encapsulation::Encapsulation),

    /// The `from` units must not import the `to` units.
    Forbidden(forbidden::Forbidden),

    /// The modules must not depend on one another.
    Independence(independence::Independence),

    /// Higher layers may import lower layers, never the reverse.
    Layers(layers::Layers),

    /// Only the allowed importers may directly import the modules.
    Protected(protected::Protected),
}

impl Rule {
    /// Compile the contract type `kind`, resolving its selectors with `units`,
    /// which collects the errors.
    pub(crate) fn compile(kind: &ContractKind, units: &mut UnitResolver) -> Self {
        match kind {
            ContractKind::Acyclic(config) => {
                Self::Acyclic(acyclic::Acyclic::compile(config, units))
            }
            ContractKind::Encapsulation(config) => {
                Self::Encapsulation(encapsulation::Encapsulation::compile(config, units))
            }
            ContractKind::Forbidden(config) => {
                Self::Forbidden(forbidden::Forbidden::compile(config, units))
            }
            ContractKind::Independence(config) => {
                Self::Independence(independence::Independence::compile(config, units))
            }
            ContractKind::Layers(config) => Self::Layers(layers::Layers::compile(config, units)),
            ContractKind::Protected(config) => {
                Self::Protected(protected::Protected::compile(config, units))
            }
        }
    }

    /// Check the rule against `view`. Links keep only the statements that
    /// aren't of the `excluded` kinds.
    pub(crate) fn check(&self, view: &GraphView, excluded: KindSet) -> Vec<Violation> {
        match self {
            Self::Acyclic(rule) => rule.check(view, excluded),
            Self::Encapsulation(rule) => rule.check(view, excluded),
            Self::Forbidden(rule) => rule.check(view, excluded),
            Self::Independence(rule) => rule.check(view, excluded),
            Self::Layers(rule) => rule.check(view, excluded),
            Self::Protected(rule) => rule.check(view, excluded),
        }
    }
}
