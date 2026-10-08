//! `independence`: the modules must not depend on one another.

use zenith_config::IndependenceConfig;
use zenith_graph::{
    GraphView, KindSet, Level, PackageDependency, find_illegal_dependencies_for_layers,
    find_illegal_direct_dependencies_for_layers,
};
use zenith_index::Index;

use super::{ensure_disjoint, layers::route_to_chain, resolve_units};
use crate::{findings::Violation, selector::Unit};

#[derive(Debug, Clone)]
pub(crate) struct Independence {
    modules: Vec<Unit>,
    allow_indirect: bool,
}

impl Independence {
    pub(crate) fn compile(index: &Index, config: &IndependenceConfig) -> Result<Self, String> {
        let modules = resolve_units(index, "modules", &config.modules)?;
        ensure_disjoint("modules", &modules)?;
        Ok(Self { modules, allow_indirect: config.allow_indirect_imports })
    }

    pub(crate) fn check(&self, view: &GraphView, kinds: KindSet) -> Vec<Violation> {
        // A single level of independent layers, as in import-linter.
        let levels = [Level {
            layers: self.modules.iter().map(|unit| unit.members.clone()).collect(),
            independent: true,
            closed: false,
        }];
        let dependencies = if self.allow_indirect {
            find_illegal_direct_dependencies_for_layers(view, &levels)
        } else {
            find_illegal_dependencies_for_layers(view, &levels)
        };

        dependencies
            .into_iter()
            .map(|PackageDependency { importer, imported, routes }| Violation::Dependency {
                importer: self.modules[importer.layer].label.clone(),
                imported: self.modules[imported.layer].label.clone(),
                chains: routes.iter().map(|route| route_to_chain(view, kinds, route)).collect(),
            })
            .collect()
    }
}
