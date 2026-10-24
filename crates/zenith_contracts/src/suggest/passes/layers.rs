//! Suggesting a `layers` contract, when the modules that take part in a
//! dependency stack in one strict order, each depending on the next.

use zenith_config::{Languages, Layer};
use zenith_diagnostics::message;
use zenith_utils::toml_writing::array;

use super::{Context, Pass};
use crate::Suggestion;

/// Suggests that the modules keep the order they stack in.
pub(super) struct Layers;

impl Pass for Layers {
    fn propose(&self, context: &Context) -> Vec<Suggestion> {
        let Some(order) = context.units.strict_order() else {
            return Vec::new();
        };
        let modules = &context.modules;
        let selectors = modules.selectors(order);
        if !selectors.iter().all(|selector| is_plain_layer(selector, context.languages)) {
            return Vec::new();
        }
        let reason = message!(contracts_suggest_layers, modules = modules.kind.noun());
        let id = format!("{}-layers", modules.kind.id());
        vec![Suggestion::new(id, reason, |table| {
            table.push_str("type = \"layers\"\n");
            table.push_str(&array("layers", &selectors));
        })]
    }
}

/// Whether `selector`, written as a layer, is a layer of just that selector,
/// rather than of the members or optional member that `|`, ` : ` or
/// parentheses in a directory's name would make of it.
fn is_plain_layer(selector: &str, languages: Languages) -> bool {
    Layer::parse(selector, false, languages).is_ok_and(|layer| {
        matches!(layer.members.as_slice(), [member]
            if !member.optional && member.selector.as_str() == selector)
    })
}

#[cfg(test)]
mod tests {
    use zenith_pipeline::LANGUAGES;

    use super::*;

    #[test]
    fn only_selectors_that_are_one_layer_as_written_are_plain() {
        assert!(is_plain_layer("[locale]", LANGUAGES));
        assert!(!is_plain_layer("(shared)", LANGUAGES));
        assert!(!is_plain_layer("a | b", LANGUAGES));
    }
}
