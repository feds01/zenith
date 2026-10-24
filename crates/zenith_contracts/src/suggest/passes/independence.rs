//! Suggesting `independence` contracts, one for each set of siblings, the
//! modules in one directory, none of which depends on another.

use zenith_config::ContractConfig;
use zenith_diagnostics::message;
use zenith_utils::toml_writing::array;

use super::{Context, Pass};
use crate::{Suggestion, suggest::modules::ModuleKind};

/// Suggests that sibling modules stay independent of each other.
pub(super) struct Independence;

impl Pass for Independence {
    fn propose(&self, context: &Context) -> Vec<Suggestion> {
        let modules = &context.modules;
        let mut suggestions = Vec::new();
        for (dir, siblings) in modules.siblings() {
            let independent = siblings.iter().all(|unit| {
                let reached = context.units.reachable_from(*unit);
                !reached.iter().any(|other| other != unit && siblings.contains(other))
            });
            if !independent {
                continue;
            }
            let id = match (modules.kind, dir) {
                (ModuleKind::Packages, "") => "top-level-packages-independent".to_string(),
                (ModuleKind::Packages, dir) => {
                    format!("{}-independent", ContractConfig::id_part(dir))
                }
                (ModuleKind::Directories, _) => "directories-independent".to_string(),
            };
            let noun = modules.kind.siblings_noun(dir);
            let reason = message!(contracts_suggest_independence, modules = noun);
            let selectors = modules.selectors(siblings);
            suggestions.push(Suggestion::new(id, reason, |table| {
                table.push_str("type = \"independence\"\n");
                table.push_str(&array("modules", &selectors));
            }));
        }
        suggestions
    }
}
