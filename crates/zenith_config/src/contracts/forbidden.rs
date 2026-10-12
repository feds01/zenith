//! The options of a `forbidden` contract.

use crate::selector::Selector;

/// The options of a `forbidden` contract: the `from` modules must not import
/// the `to` modules.
#[derive(Debug, Clone)]
pub struct ForbiddenConfig {
    /// The modules that must not import the `to` modules.
    pub from: Vec<Selector>,

    /// The modules that must not be imported.
    pub to: Vec<Selector>,

    /// Whether only direct imports count, rather than chains of imports.
    pub allow_indirect_imports: bool,

    /// The modules that chains of imports may start or end at, but not pass
    /// through, such as `"use server"` files.
    pub boundaries: Vec<Selector>,
}
