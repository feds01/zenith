//! The `independence` contract type: the modules must not depend on one
//! another.

use zenith_config::IndependenceConfig;
use zenith_graph::{Architecture, GraphView, Level, Unit};

use crate::{Chain, Violation, contract::UnitResolver, view::Statements};

/// A compiled `independence` contract.
#[derive(Debug)]
pub(crate) struct Independence {
    /// The units that must not depend on one another, which don't overlap.
    modules: Vec<Unit>,

    /// The units as one independent level of layers, as in import-linter.
    architecture: Architecture,

    /// Whether only direct imports count, rather than chains of imports.
    allow_indirect: bool,
}

impl Independence {
    /// Compile `config`, resolving its selectors with `units`. Modules that
    /// overlap are an error.
    pub(crate) fn compile(config: &IndependenceConfig, units: &mut UnitResolver) -> Self {
        let located = units.located_units(&config.modules);
        units.ensure_apart(&located);
        let modules: Vec<_> = located.into_iter().map(|(unit, _)| unit).collect();
        let level = Level {
            layers: modules.iter().map(|unit| unit.members.clone()).collect(),
            independent: true,
            closed: false,
        };
        let architecture = Architecture::new([level]);
        Self { modules, architecture, allow_indirect: config.allow_indirect_imports }
    }

    /// The dependencies between the modules in `view`, one violation per
    /// ordered pair. A chain through a third module is reported as two
    /// dependencies, one on each side of it.
    pub(crate) fn check(&self, view: &GraphView, statements: &Statements) -> Vec<Violation> {
        let dependencies = if self.allow_indirect {
            view.find_illegal_direct_dependencies_for_layers(&self.architecture)
        } else {
            view.find_illegal_dependencies_for_layers(&self.architecture)
        };
        let label = |layer: zenith_graph::LayerRef| self.modules[layer.layer.index()].label.clone();
        dependencies
            .into_iter()
            .map(|dependency| Violation::Dependency {
                importer: label(dependency.importer),
                imported: label(dependency.imported),
                chains: dependency
                    .routes
                    .iter()
                    .map(|route| Chain::from_route(view, statements, route))
                    .collect(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ContractErrorKind,
        testing::{Project, values},
    };

    #[test]
    fn modules_may_not_depend_on_each_other_in_either_direction() {
        let project = Project::new(
            &values(&[
                ("src/a/x.ts", "src/b/y.ts"),
                ("src/b/y.ts", "src/c/z.ts"),
                // Through a module outside the contract.
                ("src/c/w.ts", "lib/util.ts"),
                ("lib/util.ts", "src/a/x.ts"),
            ]),
            &[],
            "[contracts.i]\ntype = \"independence\"\nmodules = [\"src/*\"]\n",
        );
        assert_eq!(
            project.findings("i"),
            [
                "src/a -> src/b: src/a/x.ts -> src/b/y.ts",
                "src/b -> src/c: src/b/y.ts -> src/c/z.ts",
                "src/c -> src/a: src/c/w.ts -> lib/util.ts -> src/a/x.ts",
            ]
        );
    }

    #[test]
    fn a_chain_through_a_third_module_is_two_violations() {
        let project = Project::new(
            &values(&[("src/a/x.ts", "src/c/z.ts"), ("src/c/z.ts", "src/b/y.ts")]),
            &[],
            "[contracts.i]\ntype = \"independence\"\nmodules = [\"src/*\"]\n",
        );
        assert_eq!(
            project.findings("i"),
            [
                "src/a -> src/c: src/a/x.ts -> src/c/z.ts",
                "src/c -> src/b: src/c/z.ts -> src/b/y.ts",
            ]
        );
    }

    #[test]
    fn allowing_indirect_imports_only_counts_direct_ones() {
        let project = Project::new(
            &values(&[("src/a/x.ts", "lib/util.ts"), ("lib/util.ts", "src/b/y.ts")]),
            &[],
            "[contracts.i]\ntype = \"independence\"\nmodules = [\"src/*\"]\nallow_indirect_imports = true\n",
        );
        assert!(project.check("i").is_kept());
    }

    #[test]
    fn chains_do_not_pass_through_boundaries() {
        let project = Project::new(
            &values(&[
                ("src/a/x.ts", "lib/actions.ts"),
                ("lib/actions.ts", "src/b/y.ts"),
                ("src/a/x.ts", "lib/util.ts"),
                ("lib/util.ts", "src/c/z.ts"),
            ]),
            &[],
            include_str!("fixtures/independence-boundaries.toml"),
        );
        assert_eq!(
            project.findings("i"),
            ["src/a -> src/c: src/a/x.ts -> lib/util.ts -> src/c/z.ts"]
        );
    }

    #[test]
    fn imports_into_a_shared_module_can_be_ignored() {
        let toml = "[contracts.i]
type = \"independence\"
modules = [\"features/*\"]
ignore_imports = [\"features/* -> features/shared\"]
";
        let project = Project::new(
            &values(&[
                ("features/checkout/a.ts", "features/shared/s.ts"),
                ("features/catalog/b.ts", "features/shared/s.ts"),
                // Still flagged: `shared` importing a feature.
                ("features/shared/s.ts", "features/catalog/c.ts"),
            ]),
            &[],
            toml,
        );
        assert_eq!(
            project.findings("i"),
            ["features/shared -> features/catalog: features/shared/s.ts -> features/catalog/c.ts"]
        );
    }

    #[test]
    fn overlapping_modules_are_an_error() {
        let toml = "[contracts.i]\ntype = \"independence\"\nmodules = [\"src\", \"src/a\"]\n";
        let project = Project::new(&values(&[("src/a/x.ts", "src/b/y.ts")]), &[], toml);
        let errors = project.compile().unwrap_err();
        let kinds: Vec<_> = errors.iter().map(|error| &error.kind).collect();
        let expected = ContractErrorKind::OverlappingModules {
            first: "src".to_string(),
            second: "src/a".to_string(),
        };
        assert_eq!(kinds, [&expected]);
        let range = errors[0].range.unwrap();
        assert_eq!(&toml[range.start() as usize..range.end() as usize], "\"src/a\"");
    }
}
