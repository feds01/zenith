//! The layers of a `layers` contract.
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
//! A layer written as a table can also be `closed`.

use std::fmt;

use crate::selector::{Selector, SelectorError};

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
}

/// A member of a layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerMember {
    /// The modules of the member.
    pub selector: Selector,

    /// Whether the member may match nothing, written `(a)`.
    pub optional: bool,
}

/// Why a layer is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerError {
    /// The layer separates its members with both `|` and `:`.
    MixedSeparators,

    /// A member is empty, as in `"a | "`.
    EmptyMember,

    /// A member is not a valid selector.
    Selector {
        /// The member as written.
        member: String,

        /// What is wrong with it.
        error: SelectorError,
    },
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MixedSeparators => f.write_str("a layer cannot mix `|` and `:` separators"),
            Self::EmptyMember => f.write_str("a member is empty"),
            Self::Selector { member, error } => write!(f, "invalid selector `{member}`: {error}"),
        }
    }
}

impl std::error::Error for LayerError {}

impl Layer {
    /// Parse the string form of a layer.
    pub fn parse(raw: &str, closed: bool) -> Result<Self, LayerError> {
        let colon_parts = split_spaced_colons(raw);
        let has_pipe = raw.contains('|');
        let has_colon = colon_parts.len() > 1;
        if has_pipe && has_colon {
            return Err(LayerError::MixedSeparators);
        }
        let parts = if has_pipe { raw.split('|').collect() } else { colon_parts };

        let members = parts
            .into_iter()
            .map(|part| {
                let part = part.trim();
                if part.is_empty() {
                    return Err(LayerError::EmptyMember);
                }
                let (member, optional) =
                    match part.strip_prefix('(').and_then(|p| p.strip_suffix(')')) {
                        Some(inner) => (inner.trim(), true),
                        None => (part, false),
                    };
                let selector = Selector::parse(member)
                    .map_err(|error| LayerError::Selector { member: member.to_string(), error })?;
                Ok(LayerMember { selector, optional })
            })
            .collect::<Result<_, _>>()?;

        Ok(Self { members, independent: !has_colon, closed })
    }
}

/// Split a layer at each `:` that has whitespace on both sides.
fn split_spaced_colons(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, _) in raw.match_indices(':') {
        let before = raw[..index].chars().next_back();
        let after = raw[index + 1..].chars().next();
        if before.is_some_and(char::is_whitespace) && after.is_some_and(char::is_whitespace) {
            parts.push(&raw[start..index]);
            start = index + 1;
        }
    }
    parts.push(&raw[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The members of a layer as `(selector, optional)` pairs.
    fn members(layer: &Layer) -> Vec<(&str, bool)> {
        layer.members.iter().map(|member| (member.selector.as_str(), member.optional)).collect()
    }

    #[test]
    fn parses_a_single_layer() {
        let layer = Layer::parse("apps/*", false).unwrap();
        assert_eq!(members(&layer), [("apps/*", false)]);
        assert!(layer.independent);
        assert!(!layer.closed);
        assert!(Layer::parse("pkg:@acme/api", true).unwrap().closed);
    }

    #[test]
    fn parses_independent_siblings() {
        let layer = Layer::parse("pkg:web | pkg:sdk|apps/* | (services/*)", false).unwrap();
        assert_eq!(
            members(&layer),
            [("pkg:web", false), ("pkg:sdk", false), ("apps/*", false), ("services/*", true)]
        );
        assert!(layer.independent);
    }

    #[test]
    fn parses_siblings_that_may_import_each_other() {
        let layer =
            Layer::parse("pkg:@acme/types : pkg:@acme/config\t:\t(npm:zod)", false).unwrap();
        assert_eq!(
            members(&layer),
            [("pkg:@acme/types", false), ("pkg:@acme/config", false), ("npm:zod", true)]
        );
        assert!(!layer.independent);
    }

    #[test]
    fn colons_without_spaces_belong_to_the_selector() {
        let layer = Layer::parse("pkg:web: npm:a :b", false).unwrap();
        assert_eq!(members(&layer), [("pkg:web: npm:a :b", false)]);
    }

    #[test]
    fn rejects_invalid_layers() {
        assert_eq!(Layer::parse("a | b : c", false), Err(LayerError::MixedSeparators));
        assert_eq!(Layer::parse("a | ", false), Err(LayerError::EmptyMember));
        assert_eq!(Layer::parse(" : a", false), Err(LayerError::EmptyMember));
        assert_eq!(Layer::parse("", false), Err(LayerError::EmptyMember));
        assert_eq!(
            Layer::parse("a | /b", false),
            Err(LayerError::Selector {
                member: "/b".into(),
                error: SelectorError::Pattern(zenith_utils::glob::GlobError::LeadingSlash)
            })
        );
        assert_eq!(
            Layer::parse("a | ()", false).unwrap_err().to_string(),
            "invalid selector ``: the pattern is empty"
        );
    }
}
