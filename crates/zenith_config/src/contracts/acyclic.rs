//! The options of an `acyclic` contract.

use crate::selector::Selector;

/// The options of an `acyclic` contract: the modules must not form dependency
/// cycles.
#[derive(Debug, Clone)]
pub struct AcyclicConfig {
    /// The modules that must not form dependency cycles.
    pub modules: Vec<Selector>,

    /// How many further generations of children to check as their own sets
    /// of modules.
    pub depth: u32,

    /// Modules whose children are not checked.
    pub skip: Vec<Selector>,
}
