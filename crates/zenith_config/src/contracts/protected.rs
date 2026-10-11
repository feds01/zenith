//! The options of a `protected` contract.

use crate::selector::Selector;

/// The options of a `protected` contract: only the allowed importers may
/// directly import the modules.
#[derive(Debug, Clone)]
pub struct ProtectedConfig {
    /// The protected modules.
    pub modules: Vec<Selector>,

    /// The only modules that may directly import the protected modules,
    /// besides the protected modules themselves.
    pub allowed_importers: Vec<Selector>,
}
