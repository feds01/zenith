//! `protected`: only the allowed importers may directly import the modules.

use zenith_config::ProtectedConfig;
use zenith_graph::{GraphView, KindSet, NodeSet};
use zenith_index::Index;

use super::{all_members, resolve_units};
use crate::{
    findings::{Link, Violation},
    selector::Unit,
};

#[derive(Debug, Clone)]
pub(crate) struct Protected {
    modules: Vec<Unit>,
    allowed: NodeSet,
}

impl Protected {
    pub(crate) fn compile(index: &Index, config: &ProtectedConfig) -> Result<Self, String> {
        Ok(Self {
            modules: resolve_units(index, "modules", &config.modules)?,
            allowed: all_members(&resolve_units(
                index,
                "allowed_importers",
                &config.allowed_importers,
            )?),
        })
    }

    pub(crate) fn check(&self, view: &GraphView, kinds: KindSet) -> Vec<Violation> {
        let mut violations = Vec::new();
        for unit in &self.modules {
            let mut imports = Vec::new();
            for member in unit.members.iter() {
                for importer in view.importers_of(member) {
                    if !self.allowed.contains(importer) && !unit.members.contains(importer) {
                        imports.push(Link::new(view, kinds, importer, member));
                    }
                }
            }
            if !imports.is_empty() {
                imports.sort_by(|a, b| (&a.importer, &a.imported).cmp(&(&b.importer, &b.imported)));
                violations.push(Violation::IllegalImports { module: unit.label.clone(), imports });
            }
        }
        violations
    }
}
