//! The options of an `encapsulation` contract.

use zenith_utils::glob::Glob;

use crate::selector::Selector;

/// The options of an `encapsulation` contract: the modules may only be
/// imported through their entry points.
#[derive(Debug, Clone)]
pub struct EncapsulationConfig {
    /// The encapsulated modules.
    pub modules: Vec<Selector>,

    /// Globs, relative to each module, of the files that may be imported from
    /// outside it. Defaults to `index`.
    pub entrypoints: Vec<Glob>,

    /// Modules that may import any file of the encapsulated modules.
    pub allowed_importers: Vec<Selector>,
}
