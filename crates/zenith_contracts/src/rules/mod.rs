//! The contract types: what each one checks, with its units resolved.

mod forbidden;
mod protected;

use zenith_config::ContractKind;
use zenith_graph::{GraphView, KindSet};

use crate::{ContractErrorKind, Violation, contract::UnitResolver};

/// What a contract checks: one variant per contract type, with the units
/// that its selectors resolved to.
#[derive(Debug)]
pub(crate) enum Rule {
    /// The `from` units must not import the `to` units.
    Forbidden(forbidden::Forbidden),

    /// Only the allowed importers may directly import the modules.
    Protected(protected::Protected),
}

impl Rule {
    /// Compile the contract type `kind`, resolving its selectors with `units`,
    /// which collects the errors. `None` if the type isn't supported yet.
    pub(crate) fn compile(kind: &ContractKind, units: &mut UnitResolver) -> Option<Self> {
        match kind {
            ContractKind::Forbidden(config) => {
                Some(Self::Forbidden(forbidden::Forbidden::compile(config, units)))
            }
            ContractKind::Protected(config) => {
                Some(Self::Protected(protected::Protected::compile(config, units)))
            }
            ContractKind::Independence(_)
            | ContractKind::Layers(_)
            | ContractKind::Acyclic(_)
            | ContractKind::Encapsulation(_) => {
                units.error(ContractErrorKind::Unsupported { kind: kind.type_name() }, None);
                None
            }
        }
    }

    /// Check the rule against `view`. Links keep only the statements that
    /// aren't of the `excluded` kinds.
    pub(crate) fn check(&self, view: &GraphView, excluded: KindSet) -> Vec<Violation> {
        match self {
            Self::Forbidden(rule) => rule.check(view, excluded),
            Self::Protected(rule) => rule.check(view, excluded),
        }
    }
}
