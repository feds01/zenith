//! The options of a `layers` contract, and its layers.
//!
//! A layer is written as a string of one or more members:
//!
//! - `"a"` is a single layer. Members are selectors, so `"apps/*"` works.
//! - `"a | b"` puts independent siblings in one layer: they may not import each
//!   other. `|` may be written with or without spaces.
//! - `"a : b"` puts siblings in one layer that may import each other. `:` only
//!   separates members when surrounded by whitespace, because selectors such as
//!   `pkg:a` contain it too.
//! - `"(a)"` makes a member optional, so a container may lack it.
//!
//! A layer written as a table can also be `closed`. [`Layer::parse`] parses
//! the string.

use zenith_utils::{glob::NamePattern, location::ByteRange};

use crate::selector::Selector;

/// The options of a `layers` contract: higher layers may import lower layers,
/// never the reverse.
#[derive(Debug, Clone)]
pub struct LayersConfig {
    /// The layers, from highest to lowest. There is at least one. With
    /// `containers`, every member is a path selector relative to each
    /// container.
    pub layers: Vec<Layer>,

    /// The containers that the layers are relative to, each checked on its
    /// own. Empty when the layers are relative to the project root.
    pub containers: Vec<Selector>,

    /// Whether every child of every container must be declared as a layer.
    /// Only set when there are containers.
    pub exhaustive: bool,

    /// The children exempt from the exhaustiveness check, matched against
    /// the base name with or without its JS/TS extension.
    pub exhaustive_ignores: Vec<NamePattern>,

    /// Whether only direct imports count, rather than chains of imports.
    pub allow_indirect_imports: bool,

    /// The modules that chains of imports may start or end at, but not pass
    /// through, such as `"use server"` files.
    pub boundaries: Vec<Selector>,
}

/// One layer of a `layers` contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// The members of the layer, in the order written. There is at least
    /// one.
    pub members: Vec<LayerMember>,

    /// Whether the members are independent siblings that may not import each
    /// other (`|`), rather than siblings that may (`:`).
    pub independent: bool,

    /// Whether the layer is closed: the layers above it may not import the
    /// layers below it.
    pub closed: bool,

    /// The byte range that the layer is written at in the configuration file,
    /// for errors found when resolving it. Its members' selectors have the
    /// same range. `None` for a layer that wasn't read from a file.
    pub range: Option<ByteRange>,
}

/// A member of a layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerMember {
    /// The modules of the member.
    pub selector: Selector,

    /// Whether the member may match nothing, written `(a)`.
    pub optional: bool,
}

impl Layer {
    /// Record where the layer, and so its members, is written.
    pub(crate) fn at(self, range: ByteRange) -> Self {
        let members = self
            .members
            .into_iter()
            .map(|member| LayerMember { selector: member.selector.at(range), ..member })
            .collect();
        Self { members, range: Some(range), ..self }
    }
}
