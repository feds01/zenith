//! The `protected` contract type: only the allowed importers may directly
//! import the modules.

use zenith_config::ProtectedConfig;
use zenith_graph::{GraphView, NodeSet};

use crate::{
    Link, Violation,
    selection::{ImportedUnit, UnitResolver},
    view::Statements,
};

/// A compiled `protected` contract.
#[derive(Debug)]
pub(crate) struct Protected {
    /// The protected units, with the names that an import of each must take
    /// to count.
    modules: Vec<ImportedUnit>,

    /// The members of every allowed importer.
    allowed: NodeSet,
}

impl Protected {
    /// Compile `config`, resolving its selectors with `units`.
    pub(crate) fn compile(config: &ProtectedConfig, units: &mut UnitResolver) -> Self {
        let modules = units.imported_units(&config.modules);
        let mut allowed = NodeSet::new();
        for unit in units.units(&config.allowed_importers) {
            allowed.union_with(&unit.members);
        }
        Self { modules, allowed }
    }

    /// The direct imports of each protected unit by code that is neither an
    /// allowed importer nor a member of the unit, one violation per unit, in
    /// the order of the units. Indirect imports don't count.
    pub(crate) fn check(&self, view: &GraphView, statements: &Statements) -> Vec<Violation> {
        let mut violations = Vec::new();
        for module in &self.modules {
            let unit = &module.unit;
            let mut imports = Vec::new();
            for member in unit.members.iter() {
                for importer in view.importers_of(member) {
                    if !self.allowed.contains(importer) && !unit.members.contains(importer) {
                        imports.push(Link::new(view, statements, importer, member));
                    }
                }
            }
            // With `names`, only the statements that take one of them count.
            if module.names.is_some() {
                for link in &mut imports {
                    link.imports.retain(|detail| module.takes(detail));
                }
                imports.retain(|link| !link.imports.is_empty());
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
    use crate::testing::{Project, values};

    #[test]
    fn only_allowed_importers_and_members_may_import_directly() {
        let project = Project::new(
            &values(&[
                // Flagged: a component importing an agent directly.
                ("web/components/chat.tsx", "web/agents/support.ts"),
                ("web/components/chat.tsx", "web/agents/tools.ts"),
                // Allowed: an allowed importer, and a member.
                ("web/app/api/route.ts", "web/agents/support.ts"),
                ("web/agents/support.ts", "web/agents/tools.ts"),
                // Not checked: an indirect import.
                ("web/components/run.tsx", "web/lib/run.ts"),
                ("web/lib/run.ts", "web/agents/support.ts"),
            ]),
            &[],
            "[contracts.p]\ntype = \"protected\"\nmodules = [\"web/agents\"]\nallowed_importers = [\"web/app/api\"]\n",
        );
        assert_eq!(
            project.findings("p"),
            [
                "web/agents: web/components/chat.tsx -> web/agents/support.ts",
                "web/agents: web/components/chat.tsx -> web/agents/tools.ts",
                "web/agents: web/lib/run.ts -> web/agents/support.ts",
            ]
        );
    }

    #[test]
    fn names_keep_the_statements_that_take_one_of_them() {
        let project = Project::with_names(
            &[
                ("web/components/chat.tsx", "web/agents/support.ts", "Message"),
                ("web/components/chat.tsx", "web/agents/support.ts", "run"),
                ("web/components/list.tsx", "web/agents/support.ts", "Message"),
                ("web/app/api/route.ts", "web/agents/support.ts", "run"),
            ],
            include_str!("fixtures/protected-names.toml"),
        );
        let report = project.check("p");
        assert_eq!(
            project.findings("p"),
            ["web/agents: web/components/chat.tsx -> web/agents/support.ts"]
        );
        let crate::Violation::IllegalImports { imports, .. } = &report.violations[0] else {
            panic!("expected illegal imports");
        };
        let starts: Vec<_> = imports[0].imports.iter().map(|detail| detail.range.start()).collect();
        assert_eq!(starts, [200]);
    }

    #[test]
    fn a_protected_module_without_illegal_importers_is_kept() {
        let project = Project::new(
            &values(&[("web/app/api/route.ts", "web/agents/support.ts")]),
            &[],
            "[contracts.p]\ntype = \"protected\"\nmodules = [\"web/agents\"]\nallowed_importers = [\"web/app/api\"]\n",
        );
        assert!(project.check("p").is_kept());
    }
}
