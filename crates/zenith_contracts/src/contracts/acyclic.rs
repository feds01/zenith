//! `acyclic`: no dependency cycles between the modules, optionally drilling
//! down into their children.

use zenith_config::AcyclicConfig;
use zenith_graph::{
    GraphView, KindSet, ModuleId, NodeKind, UnitGraph, find_unit_path, nominate_cycle_breakers,
};
use zenith_index::Index;

use super::{ensure_disjoint, resolve_units};
use crate::{
    findings::{CycleBreaker, Link, Violation},
    selector::Unit,
};

#[derive(Debug, Clone)]
pub(crate) struct Acyclic {
    modules: Vec<Unit>,
    /// A label for the set of modules, e.g. `pkg:*`.
    scope: String,
    depth: u32,
    skip: Vec<ModuleId>,
}

impl Acyclic {
    pub(crate) fn compile(index: &Index, config: &AcyclicConfig) -> Result<Self, String> {
        let modules = resolve_units(index, "modules", &config.modules)?;
        ensure_disjoint("modules", &modules)?;
        let skip = resolve_units(index, "skip", &config.skip)?
            .into_iter()
            .filter_map(|unit| unit.root)
            .collect();
        let scope = config
            .modules
            .iter()
            .map(|selector| match selector {
                zenith_config::SelectorConfig::Pattern(raw) => raw.clone(),
                zenith_config::SelectorConfig::Table(table) => table.path.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        Ok(Self { modules, scope, depth: config.depth, skip })
    }

    pub(crate) fn check(&self, view: &GraphView, kinds: KindSet) -> Vec<Violation> {
        let mut violations = Vec::new();
        self.check_level(
            view,
            kinds,
            self.scope.clone(),
            &self.modules,
            self.depth,
            &mut violations,
        );
        violations
    }

    fn check_level(
        &self,
        view: &GraphView,
        kinds: KindSet,
        scope: String,
        units: &[Unit],
        depth: u32,
        violations: &mut Vec<Violation>,
    ) {
        if units.len() >= 2 {
            let members: Vec<_> = units.iter().map(|unit| unit.members.clone()).collect();
            let labels: Vec<_> = units.iter().map(|unit| unit.label.clone()).collect();
            let unit_graph = UnitGraph::build(view, &members);
            let breakers = nominate_cycle_breakers(&unit_graph, &labels);

            if !breakers.is_empty() {
                let breakers = breakers
                    .into_iter()
                    .map(|(from, to)| {
                        let mut imports = Vec::new();
                        for importer in units[from].members.iter() {
                            for imported in view.imports_of(importer) {
                                if units[to].members.contains(imported) {
                                    imports.push(Link::new(view, kinds, importer, imported));
                                }
                            }
                        }
                        let mut cycle = vec![labels[from].clone()];
                        if let Some(path) = find_unit_path(&unit_graph, to, from) {
                            cycle.extend(path.into_iter().map(|unit| labels[unit].clone()));
                        }
                        CycleBreaker {
                            importer: labels[from].clone(),
                            imported: labels[to].clone(),
                            imports,
                            cycle,
                        }
                    })
                    .collect();
                violations.push(Violation::Cycle { scope, breakers });
            }
        }

        if depth == 0 {
            return;
        }
        let graph = view.graph();
        for unit in units {
            let Some(root) = unit.root else { continue };
            if graph.node(root).kind != NodeKind::Directory || self.skip.contains(&root) {
                continue;
            }
            let children: Vec<Unit> = graph
                .children(root)
                .iter()
                .map(|child| {
                    let mut members = graph.members(*child);
                    // Respect `except` filters applied to the parent unit.
                    members.difference_with(&{
                        let mut outside = graph.members(*child);
                        outside.difference_with(&unit.members);
                        outside
                    });
                    Unit { label: graph.name(*child).to_string(), root: Some(*child), members }
                })
                .filter(|child| !child.members.is_empty())
                .collect();
            self.check_level(
                view,
                kinds,
                format!("children of {}", unit.label),
                &children,
                depth - 1,
                violations,
            );
        }
    }
}
