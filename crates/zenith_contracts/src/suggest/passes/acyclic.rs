//! Suggesting an `acyclic` contract, when the modules depend on each other in
//! no cycle, with the packages written as `pkg:*` when that selects just
//! them.

use zenith_diagnostics::message;
use zenith_utils::toml_writing::array;

use super::{Context, Pass};
use crate::Suggestion;

/// Suggests that the modules stay free of cycles.
pub(super) struct Acyclic;

impl Pass for Acyclic {
    fn propose(&self, context: &Context) -> Vec<Suggestion> {
        if !context.units.nominate_cycle_breakers().is_empty() {
            return Vec::new();
        }
        let modules = &context.modules;
        let reason = message!(contracts_suggest_acyclic, modules = modules.kind.noun());
        let selectors = modules.all_selectors();
        let id = format!("{}-acyclic", modules.kind.id());
        vec![Suggestion::new(id, reason, |table| {
            table.push_str("type = \"acyclic\"\n");
            table.push_str(&array("modules", &selectors));
        })]
    }
}
