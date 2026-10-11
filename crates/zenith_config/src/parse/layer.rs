//! Parsing the strings that layers are written as, and the layers of a
//! `layers` contract.

use super::Parser;
use crate::{
    contracts::{Layer, LayerMember},
    error::{ConfigErrorKind, LayerError},
    schema::{LayerValue, Located},
    selector::{Selector, SelectorTarget},
};

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

        Ok(Self { members, independent: !has_colon, closed, range: None })
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

impl Parser {
    /// Parse a layer of a `layers` contract. With `relative`, its members
    /// must be paths, relative to each container.
    pub(super) fn parse_layer(
        &mut self,
        value: &Located<LayerValue>,
        relative: bool,
    ) -> Option<Layer> {
        let (raw, range, closed) = match &value.value {
            LayerValue::String(raw) => (raw, value.range, false),
            LayerValue::Table(table) => (&table.layer.value, table.layer.range, table.closed),
        };
        let layer = Layer::parse(raw, closed).and_then(|layer| {
            let not_relative = layer.members.iter().find(|member| {
                relative && !matches!(member.selector.target(), SelectorTarget::Path(_))
            });
            match not_relative {
                Some(member) => {
                    Err(LayerError::NotRelative { member: member.selector.as_str().to_string() })
                }
                None => Ok(layer.at(range)),
            }
        });
        self.check(layer, Some(range), |error| ConfigErrorKind::InvalidLayer {
            layer: raw.clone(),
            error,
        })
    }
}

#[cfg(test)]
mod tests {
    use zenith_utils::glob::GlobError;

    use super::*;
    use crate::error::SelectorError;

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
                error: SelectorError::Pattern(GlobError::LeadingSlash)
            })
        );
        assert_eq!(
            Layer::parse("a | ()", false).unwrap_err().to_string(),
            "invalid selector ``: the pattern is empty"
        );
    }
}
