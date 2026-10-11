//! The options of an `independence` contract.

use crate::selector::Selector;

/// The options of an `independence` contract: the modules must not depend on
/// one another.
#[derive(Debug, Clone)]
pub struct IndependenceConfig {
    /// The modules that must not depend on one another.
    pub modules: Vec<Selector>,

    /// Whether only direct imports count, rather than chains of imports.
    pub allow_indirect_imports: bool,
}
