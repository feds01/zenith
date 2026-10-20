//! The `encapsulation` contract type: the modules may only be imported
//! through their entry points.

use zenith_config::{EncapsulationConfig, OPTIONAL_EXTENSIONS};
use zenith_graph::{GraphView, NodeKind, NodeSet, Unit};
use zenith_utils::glob::{Glob, escape};

use crate::{ContractErrorKind, Link, Violation, contract::UnitResolver, view::Statements};

/// A compiled `encapsulation` contract.
#[derive(Debug)]
pub(crate) struct Encapsulation {
    /// The encapsulated directories, each with its entry points.
    modules: Vec<(Unit, NodeSet)>,

    /// The members of every allowed importer.
    allowed: NodeSet,
}

impl Encapsulation {
    /// Compile `config`, resolving its selectors with `units`. A module that
    /// is not a directory is an error.
    pub(crate) fn compile(config: &EncapsulationConfig, units: &mut UnitResolver) -> Self {
        let graph = units.graph();
        let mut modules = Vec::new();
        for (unit, range) in units.located_units(&config.modules) {
            if graph.node(unit.root).kind() != NodeKind::Directory {
                units.error(ContractErrorKind::NotADirectory { module: unit.label }, range);
                continue;
            }
            // Entry points are relative to the module, and match files, and packages, which
            // are imported as a whole.
            let root = escape(graph.name(unit.root));
            let globs: Vec<_> = config
                .entrypoints
                .iter()
                .filter_map(|entry| {
                    let entry = entry.as_str().trim_start_matches("./");
                    Glob::new(&format!("{root}/{entry}")).ok()
                })
                .collect();
            let entries = unit
                .members
                .iter()
                .filter(|member| {
                    let node = graph.node(*member);
                    (node.kind() == NodeKind::File || graph.is_package(*member))
                        && globs.iter().any(|glob| node.matches_glob(glob, OPTIONAL_EXTENSIONS))
                })
                .collect();
            modules.push((unit, entries));
        }

        let mut allowed = NodeSet::new();
        for unit in units.units(&config.allowed_importers) {
            allowed.union_with(&unit.members);
        }
        Self { modules, allowed }
    }

    /// The imports of each module's members that aren't entry points, by
    /// code that is neither in the module nor an allowed importer: one
    /// violation per module, in the order of the modules.
    pub(crate) fn check(&self, view: &GraphView, statements: &Statements) -> Vec<Violation> {
        let mut violations = Vec::new();
        for (unit, entries) in &self.modules {
            let mut imports = Vec::new();
            for member in unit.members.iter().filter(|member| !entries.contains(*member)) {
                for importer in view.importers_of(member) {
                    if !unit.members.contains(importer) && !self.allowed.contains(importer) {
                        imports.push(Link::new(view, statements, importer, member));
                    }
                }
            }
            if !imports.is_empty() {
                imports.sort_by_key(|link| (link.importer, link.imported));
                violations.push(Violation::IllegalImports { module: unit.label.clone(), imports });
            }
        }
        violations
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ContractErrorKind,
        testing::{Project, values},
    };

    #[test]
    fn modules_may_only_be_imported_through_their_entry_points() {
        let toml = "[contracts.e]
type = \"encapsulation\"
modules = [\"src/feature\"]
entrypoints = [\"index\", \"public/**\"]
allowed_importers = [\"src/tests\"]
";
        let project = Project::new(
            &values(&[
                // Through the entry points.
                ("src/app/entry.ts", "src/feature/index.ts"),
                ("src/app/public.ts", "src/feature/public/api.ts"),
                // A deep import.
                ("src/app/deep.ts", "src/feature/internal/impl.ts"),
                // From inside the module, and from an allowed importer.
                ("src/feature/index.ts", "src/feature/internal/impl.ts"),
                ("src/tests/impl.test.ts", "src/feature/internal/impl.ts"),
            ]),
            &[],
            toml,
        );
        assert_eq!(
            project.findings("e"),
            ["src/feature: src/app/deep.ts -> src/feature/internal/impl.ts"]
        );
    }

    #[test]
    fn modules_must_be_directories() {
        let project = Project::new(
            &values(&[("src/a.ts", "src/b.ts")]),
            &[],
            "[contracts.e]\ntype = \"encapsulation\"\nmodules = [\"src/a.ts\"]\n",
        );
        let errors = project.compile().unwrap_err();
        assert_eq!(
            errors[0].kind,
            ContractErrorKind::NotADirectory { module: "src/a.ts".to_string() }
        );
    }
}
