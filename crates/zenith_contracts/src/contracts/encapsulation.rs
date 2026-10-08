//! `encapsulation`: modules may only be imported through their entry points.

use zenith_config::EncapsulationConfig;
use zenith_graph::{GraphView, KindSet, ModuleId, NodeKind, NodeSet};
use zenith_index::Index;
use zenith_utils::glob::{Glob, escape};

use super::{all_members, resolve_units};
use crate::{
    findings::{Link, Violation},
    selector::{Unit, node_matches},
};

#[derive(Debug, Clone)]
pub(crate) struct Encapsulation {
    /// Each module, with the set of its members that are entry points.
    modules: Vec<(Unit, NodeSet)>,
    allowed: NodeSet,
}

impl Encapsulation {
    pub(crate) fn compile(index: &Index, config: &EncapsulationConfig) -> Result<Self, String> {
        let graph = &index.graph;
        let default_entrypoints = vec!["index".to_string()];
        let entrypoints = config.entrypoints.as_ref().unwrap_or(&default_entrypoints);

        let mut modules = Vec::new();
        for unit in resolve_units(index, "modules", &config.modules)? {
            let root: Option<ModuleId> =
                unit.root.filter(|root| graph.node(*root).kind == NodeKind::Directory);
            let Some(root) = root else {
                return Err(format!("`modules` must be directories, but `{}` is not", unit.label));
            };
            let globs = entrypoints
                .iter()
                .map(|entry| {
                    Glob::new(&format!(
                        "{}/{}",
                        escape(graph.name(root)),
                        entry.trim_start_matches("./")
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let entries = unit
                .members
                .iter()
                .filter(|member| {
                    let node = graph.node(*member);
                    globs.iter().any(|glob| node_matches(glob, &node.name, node.kind))
                })
                .collect();
            modules.push((unit, entries));
        }

        Ok(Self {
            modules,
            allowed: all_members(&resolve_units(
                index,
                "allowed_importers",
                &config.allowed_importers,
            )?),
        })
    }

    pub(crate) fn check(&self, view: &GraphView, kinds: KindSet) -> Vec<Violation> {
        let mut violations = Vec::new();
        for (unit, entries) in &self.modules {
            let mut imports = Vec::new();
            for member in unit.members.iter() {
                if entries.contains(member) {
                    continue;
                }
                for importer in view.importers_of(member) {
                    if !unit.members.contains(importer) && !self.allowed.contains(importer) {
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
