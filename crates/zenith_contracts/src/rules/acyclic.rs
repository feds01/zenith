//! The `acyclic` contract type: the modules must not form dependency cycles.

use zenith_config::AcyclicConfig;
use zenith_graph::{GraphView, KindSet, NodeKind, NodeSet, Unit, UnitGraph};

use crate::{CycleBreaker, CycleScope, Link, Violation, contract::UnitResolver};

/// A compiled `acyclic` contract.
#[derive(Debug)]
pub(crate) struct Acyclic {
    /// The units that must not form cycles, which don't overlap.
    modules: Vec<Unit>,

    /// The units, as the contract's selectors are written.
    scope: CycleScope,

    /// How many further generations of children to check, each as a set of
    /// units of its own.
    depth: u32,

    /// The nodes whose children are not checked.
    skip: NodeSet,
}

impl Acyclic {
    /// Compile `config`, resolving its selectors with `units`. Modules that
    /// overlap are an error.
    pub(crate) fn compile(config: &AcyclicConfig, units: &mut UnitResolver) -> Self {
        let located = units.located_units(&config.modules);
        units.ensure_apart(&located);
        let skip = units.units(&config.skip).iter().map(|unit| unit.root).collect();
        let scope: Vec<_> = config.modules.iter().map(|selector| selector.as_str()).collect();
        Self {
            modules: located.into_iter().map(|(unit, _)| unit).collect(),
            scope: CycleScope::Modules(scope.join(", ")),
            depth: config.depth,
            skip,
        }
    }

    /// The cycles among the modules in `view`, then among the children of
    /// each directory module, and so on, down `depth` generations: one
    /// violation per set of units with cycles.
    pub(crate) fn check(&self, view: &GraphView, excluded: KindSet) -> Vec<Violation> {
        let mut violations = Vec::new();
        let scope = self.scope.clone();
        self.check_units(view, excluded, scope, &self.modules, self.depth, &mut violations);
        violations
    }

    /// Check the `units` of `scope`, then their children down `depth`
    /// generations, adding the violations to `violations`.
    fn check_units(
        &self,
        view: &GraphView,
        excluded: KindSet,
        scope: CycleScope,
        units: &[Unit],
        depth: u32,
        violations: &mut Vec<Violation>,
    ) {
        // 1. Nominate the dependencies that break the cycles, with the units in order
        //    of their labels, so that ties go to the first label, as in grimp.
        let mut ordered: Vec<&Unit> = units.iter().collect();
        ordered.sort_by(|a, b| a.label.cmp(&b.label));
        let members: Vec<NodeSet> = ordered.iter().map(|unit| unit.members.clone()).collect();
        let unit_graph = UnitGraph::build(view, &members);
        let breakers: Vec<_> = unit_graph
            .nominate_cycle_breakers()
            .into_iter()
            .map(|dependency| {
                let (importer, imported) =
                    (ordered[dependency.importer.index()], ordered[dependency.imported.index()]);
                let mut imports = Vec::new();
                for member in importer.members.iter() {
                    for target in
                        view.imports_of(member).filter(|target| imported.members.contains(*target))
                    {
                        imports.push(Link::new(view, excluded, member, target));
                    }
                }
                // The path back from the imported unit ends with the importer, which
                // starts the cycle.
                let back = unit_graph
                    .find_path(dependency.imported, dependency.importer)
                    .unwrap_or_default();
                let mut cycle = vec![importer.label.clone()];
                cycle.extend(
                    back.iter()
                        .take(back.len().saturating_sub(1))
                        .map(|unit| ordered[unit.index()].label.clone()),
                );
                CycleBreaker {
                    importer: importer.label.clone(),
                    imported: imported.label.clone(),
                    imports,
                    cycle,
                }
            })
            .collect();
        if !breakers.is_empty() {
            violations.push(Violation::Cycle { scope, breakers });
        }

        // 2. Check the children of each directory unit as units of their own, without
        //    the members that the unit leaves out.
        if depth == 0 {
            return;
        }
        let graph = view.graph();
        for unit in units {
            if graph.node(unit.root).kind() != NodeKind::Directory || self.skip.contains(unit.root)
            {
                continue;
            }
            let children: Vec<Unit> = graph
                .children(unit.root)
                .iter()
                .map(|child| {
                    let mut child = graph.unit(*child);
                    child.members = child
                        .members
                        .iter()
                        .filter(|member| unit.members.contains(*member))
                        .collect();
                    child
                })
                .filter(|child| !child.members.is_empty())
                .collect();
            let scope = CycleScope::Children(unit.label.clone());
            self.check_units(view, excluded, scope, &children, depth - 1, violations);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ContractErrorKind,
        testing::{Project, values},
    };

    /// An `acyclic` contract `c` with the given options.
    fn contract(options: &str) -> String {
        format!("[contracts.c]\ntype = \"acyclic\"\n{options}\n")
    }

    #[test]
    fn breaks_the_lightest_dependency_of_a_cycle() {
        let project = Project::new(
            &values(&[
                ("pkgs/a/x.ts", "pkgs/b/y.ts"),
                ("pkgs/a/w.ts", "pkgs/b/y.ts"),
                ("pkgs/b/y.ts", "pkgs/a/x.ts"),
            ]),
            &[],
            &contract("modules = [\"pkgs/*\"]"),
        );
        assert_eq!(
            project.findings("c"),
            ["pkgs/*: break pkgs/b -> pkgs/a (pkgs/b/y.ts -> pkgs/a/x.ts) in pkgs/b -> pkgs/a"]
        );
    }

    #[test]
    fn ties_break_alphabetically_and_the_cycle_goes_round() {
        let project = Project::new(
            &values(&[
                ("m/c/z.ts", "m/a/x.ts"),
                ("m/a/x.ts", "m/b/y.ts"),
                ("m/b/y.ts", "m/c/z.ts"),
            ]),
            &[],
            &contract("modules = [\"m/*\"]"),
        );
        assert_eq!(
            project.findings("c"),
            ["m/*: break m/c -> m/a (m/c/z.ts -> m/a/x.ts) in m/c -> m/a -> m/b"]
        );
    }

    #[test]
    fn units_without_cycles_are_kept() {
        let project = Project::new(
            &values(&[("m/a/x.ts", "m/b/y.ts"), ("m/b/y.ts", "m/c/z.ts")]),
            &[],
            &contract("modules = [\"m/*\"]"),
        );
        assert!(project.check("c").is_kept());
    }

    #[test]
    fn depth_checks_the_children_of_each_unit_too() {
        let imports = values(&[
            ("src/a/one.ts", "src/a/two.ts"),
            ("src/a/two.ts", "src/a/one.ts"),
            ("src/b/m.ts", "src/b/n/k.ts"),
            ("src/b/n/k.ts", "src/b/m.ts"),
        ]);
        let project = Project::new(&imports, &[], &contract("modules = [\"src/*\"]\ndepth = 1"));
        assert_eq!(
            project.findings("c"),
            [
                "children of src/a: break src/a/two.ts -> src/a/one.ts (src/a/two.ts -> src/a/one.ts) in src/a/two.ts -> src/a/one.ts",
                "children of src/b: break src/b/n -> src/b/m.ts (src/b/n/k.ts -> src/b/m.ts) in src/b/n -> src/b/m.ts",
            ]
        );

        // Not past the depth, and not into the skipped units.
        let shallow = Project::new(&imports, &[], &contract("modules = [\"src/*\"]"));
        assert!(shallow.check("c").is_kept());
        let skipped = Project::new(
            &imports,
            &[],
            &contract("modules = [\"src/*\"]\ndepth = 1\nskip = [\"src/a\"]"),
        );
        assert_eq!(skipped.findings("c").len(), 1);
    }

    #[test]
    fn overlapping_modules_are_an_error() {
        let project = Project::new(
            &values(&[("src/a/x.ts", "src/b/y.ts")]),
            &[],
            &contract("modules = [\"src\", \"src/a\"]"),
        );
        let errors = project.compile().unwrap_err();
        assert!(matches!(errors[0].kind, ContractErrorKind::OverlappingModules { .. }));
    }
}
