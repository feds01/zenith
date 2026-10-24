//! The options of a `limits` contract.

use crate::selector::Selector;

/// The options of a `limits` contract: each module may only be imported by
/// so many other nodes, and import so many. At least one of the limits is
/// set.
#[derive(Debug, Clone)]
pub struct LimitsConfig {
    /// The modules whose imports are counted, each on its own.
    pub modules: Vec<Selector>,

    /// The most distinct nodes outside a module that may import it, or
    /// `None` for no limit.
    pub max_fan_in: Option<usize>,

    /// The most distinct files and packages of the project outside a module
    /// that it may import, or `None` for no limit.
    pub max_fan_out: Option<usize>,
}
