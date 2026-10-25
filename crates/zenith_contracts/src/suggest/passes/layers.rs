//! Suggesting a `layers` contract, when the modules that take part in a
//! dependency stack in levels, each depending only on the levels below it.
//! The modules of a level, which don't depend on each other, are one layer of
//! independent siblings.

use zenith_config::{Languages, Layer};
use zenith_diagnostics::message;
use zenith_utils::toml_writing::array;

use super::{Context, Pass};
use crate::Suggestion;

/// Suggests that the modules keep the levels they stack in.
pub(super) struct Layers;

impl Pass for Layers {
    fn propose(&self, context: &Context) -> Vec<Suggestion> {
        let Some(levels) = context.units.levels() else {
            return Vec::new();
        };
        let modules = &context.modules;
        let mut layers = Vec::with_capacity(levels.len());
        for level in levels {
            let selectors = modules.selectors(level);
            if !is_plain_layer(&selectors, context.languages) {
                return Vec::new();
            }
            layers.push(selectors.join(" | "));
        }
        let reason = message!(contracts_suggest_layers, modules = modules.kind.noun());
        let id = format!("{}-layers", modules.kind.id());
        vec![Suggestion::new(id, reason, |table| {
            table.push_str("type = \"layers\"\n");
            table.push_str(&array("layers", &layers));
        })]
    }
}

/// Whether `selectors`, written as a layer apart by `|`, are a layer of
/// independent siblings that are just those selectors, rather than of the
/// members or optional members that `|`, ` : ` or parentheses in a
/// directory's name would make of them.
fn is_plain_layer(selectors: &[String], languages: Languages) -> bool {
    Layer::parse(&selectors.join(" | "), false, languages).is_ok_and(|layer| {
        layer.independent
            && layer.members.len() == selectors.len()
            && layer
                .members
                .iter()
                .zip(selectors)
                .all(|(member, selector)| !member.optional && member.selector.as_str() == selector)
    })
}

#[cfg(test)]
mod tests {
    use zenith_pipeline::LANGUAGES;

    use super::*;

    /// Whether the `selectors` are a plain layer.
    fn is_plain(selectors: &[&str]) -> bool {
        let selectors: Vec<_> = selectors.iter().map(ToString::to_string).collect();
        is_plain_layer(&selectors, LANGUAGES)
    }

    #[test]
    fn only_selectors_that_are_one_layer_as_written_are_plain() {
        assert!(is_plain(&["[locale]"]));
        assert!(is_plain(&["app", "pkg:@s/web"]));
        assert!(!is_plain(&["(shared)"]));
        assert!(!is_plain(&["a | b"]));
        assert!(!is_plain(&["a", "b : c"]));
    }
}
