//! The passes that propose contracts, one for each kind:
//!
//! - [`acyclic`], when the modules depend on each other in no cycle;
//! - [`layers`], when those that take part in a dependency stack in levels,
//!   each depending only on those below it;
//! - [`independence`], for each set of siblings that don't depend on each
//!   other.
//!
//! A new kind of suggestion is a new pass, added to [`PASSES`].

mod acyclic;
mod independence;
mod layers;

use zenith_config::Languages;
use zenith_graph::UnitGraph;

use super::{Modules, Suggestion};

/// The passes, in the order that their suggestions are listed.
pub(super) const PASSES: &[&dyn Pass] =
    &[&acyclic::Acyclic, &layers::Layers, &independence::Independence];

/// A pass that proposes contracts of one kind, from the modules and the
/// dependencies between them.
pub(super) trait Pass {
    /// The contracts that the dependencies between the modules of `context`
    /// allow. They are checked before they are suggested.
    fn propose(&self, context: &Context) -> Vec<Suggestion>;
}

/// What the passes propose contracts from.
pub(super) struct Context {
    /// The modules.
    pub(super) modules: Modules,

    /// The dependencies between the modules, as units in their order.
    pub(super) units: UnitGraph,

    /// The languages whose selector schemes the contracts may use.
    pub(super) languages: Languages,
}
