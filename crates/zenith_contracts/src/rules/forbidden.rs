//! The `forbidden` contract type: the `from` units must not import the `to`
//! units.

use rayon::prelude::*;
use zenith_config::ForbiddenConfig;
use zenith_graph::{GraphView, NodeSet, Unit};

use crate::{Chain, Violation, contract::UnitResolver, view::Statements};

/// A compiled `forbidden` contract.
#[derive(Debug)]
pub(crate) struct Forbidden {
    /// The units that must not import the `to` units.
    from: Vec<Unit>,

    /// The units that must not be imported.
    to: Vec<Unit>,

    /// Whether only direct imports count, rather than chains of imports.
    allow_indirect: bool,
}

impl Forbidden {
    /// Compile `config`, resolving its selectors with `units`.
    pub(crate) fn compile(config: &ForbiddenConfig, units: &mut UnitResolver) -> Self {
        Self {
            from: units.units(&config.from),
            to: units.units(&config.to),
            allow_indirect: config.allow_indirect_imports,
        }
    }

    /// The chains from each `from` unit to each `to` unit in `view`, one
    /// violation per pair of units, in the order of the units.
    pub(crate) fn check(&self, view: &GraphView, statements: &Statements) -> Vec<Violation> {
        // 1. Find everything that reaches each `to` unit, so that a pair without a
        //    chain is ruled out without a search.
        let reachers: Vec<NodeSet> = if self.allow_indirect {
            Vec::new()
        } else {
            self.to.par_iter().map(|to| view.find_downstream(&to.members)).collect()
        };

        // 2. Search every pair of units, in parallel.
        let pairs: Vec<(&Unit, usize)> = self
            .from
            .iter()
            .flat_map(|from| (0..self.to.len()).map(move |to| (from, to)))
            .collect();
        pairs
            .into_par_iter()
            .filter_map(|(from, to_index)| {
                let to = &self.to[to_index];
                // Units that overlap, e.g. a directory and a file in it, don't
                // describe an import between them.
                if from.members.is_empty() || from.members.intersects(&to.members) {
                    return None;
                }
                let mut paths = if self.allow_indirect {
                    let direct = view.direct_imports(&from.members, &to.members);
                    direct.map(|(importer, imported)| vec![importer, imported]).collect()
                } else if from.members.intersects(&reachers[to_index]) {
                    view.find_shortest_chains(&from.members, &to.members, &NodeSet::new())
                } else {
                    return None;
                };
                if paths.is_empty() {
                    return None;
                }

                // Ids are in name order, so this sorts the chains shortest first, then
                // by the names of the nodes they go through.
                paths.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
                let chains = paths.iter().map(|path| Chain::from_path(view, statements, path));
                Some(Violation::Dependency {
                    importer: from.label.clone(),
                    imported: to.label.clone(),
                    chains: chains.collect(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use zenith_graph::ImportKind;

    use crate::testing::{Project, values};

    /// A `forbidden` contract `f` with the given options.
    fn contract(options: &str) -> String {
        format!("[contracts.f]\ntype = \"forbidden\"\n{options}\n")
    }

    #[test]
    fn finds_direct_and_indirect_chains_shortest_first() {
        let project = Project::new(
            &values(&[
                ("a/w.ts", "c/z.ts"),
                ("c/z.ts", "b/y.ts"),
                ("a/x.ts", "b/y.ts"),
                ("b/y.ts", "a/x.ts"),
            ]),
            &[],
            &contract("from = [\"a\"]\nto = [\"b\"]"),
        );
        assert_eq!(
            project.findings("f"),
            ["a -> b: a/x.ts -> b/y.ts", "a -> b: a/w.ts -> c/z.ts -> b/y.ts"]
        );
    }

    #[test]
    fn allowing_indirect_imports_only_counts_direct_ones() {
        let project = Project::new(
            &values(&[("a/w.ts", "c/z.ts"), ("c/z.ts", "b/y.ts"), ("a/x.ts", "b/y.ts")]),
            &[],
            &contract("from = [\"a\"]\nto = [\"b\"]\nallow_indirect_imports = true"),
        );
        assert_eq!(project.findings("f"), ["a -> b: a/x.ts -> b/y.ts"]);
    }

    #[test]
    fn each_pair_of_units_is_a_violation_of_its_own() {
        let project = Project::new(
            &values(&[
                ("src/a/x.ts", "src/b/y.ts"),
                ("src/c/z.ts", "src/b/y.ts"),
                // `src/d` imports, but isn't imported.
                ("src/d/w.ts", "src/a/x.ts"),
            ]),
            &[],
            &contract("from = [\"src/a\", \"src/c\"]\nto = [\"src/b\", \"src/d\"]"),
        );
        let report = project.check("f");
        assert_eq!(report.violations.len(), 2);
        assert_eq!(
            project.findings("f"),
            [
                "src/a -> src/b: src/a/x.ts -> src/b/y.ts",
                "src/c -> src/b: src/c/z.ts -> src/b/y.ts"
            ]
        );
    }

    #[test]
    fn externals_and_builtins_can_be_forbidden() {
        let project = Project::new(
            &values(&[
                ("web/a.ts", "lib/images.ts"),
                ("lib/images.ts", "npm:sharp"),
                ("web/b.ts", "node:fs"),
            ]),
            &[],
            &contract("from = [\"web\"]\nto = [\"npm:sharp\", \"node:*\", \"npm:left-pad\"]"),
        );
        assert_eq!(
            project.findings("f"),
            [
                "web -> npm:sharp: web/a.ts -> lib/images.ts -> npm:sharp",
                "web -> node:fs: web/b.ts -> node:fs",
            ]
        );
    }

    #[test]
    fn a_directive_makes_each_file_a_unit() {
        let project = Project::new(
            &values(&[
                ("web/page.tsx", "web/lib.ts"),
                ("web/lib.ts", "npm:sharp"),
                ("web/route.ts", "npm:sharp"),
            ]),
            &["web/page.tsx"],
            &contract(
                "from = [{ path = \"web\", directive = \"use client\" }]\nto = [\"npm:sharp\"]",
            ),
        );
        // `web/route.ts` is not a client file.
        assert_eq!(
            project.findings("f"),
            ["web/page.tsx -> npm:sharp: web/page.tsx -> web/lib.ts -> npm:sharp"]
        );
    }

    #[test]
    fn overlapping_units_are_skipped() {
        let project = Project::new(
            &values(&[("a/x.ts", "a/y.ts")]),
            &[],
            &contract("from = [\"a\"]\nto = [\"a/*\"]"),
        );
        assert!(project.check("f").is_kept());
    }

    #[test]
    fn excluded_kinds_leave_out_imports_and_statements() {
        let imports = [
            ("a/x.ts", "b/y.ts", ImportKind::Type),
            ("a/w.ts", "b/y.ts", ImportKind::Type),
            ("a/w.ts", "b/y.ts", ImportKind::Value),
        ];
        let project = Project::new(
            &imports,
            &[],
            &contract("from = [\"a\"]\nto = [\"b\"]\nexclude_kinds = [\"type\"]"),
        );
        let report = project.check("f");
        assert_eq!(project.findings("f"), ["a -> b: a/w.ts -> b/y.ts"]);

        // Only the value import of `a/w.ts` is kept in the link.
        let crate::Violation::Dependency { chains, .. } = &report.violations[0] else {
            panic!("expected a dependency");
        };
        let kinds: Vec<_> = chains[0].links[0].imports.iter().map(|detail| detail.kind).collect();
        assert_eq!(kinds, [ImportKind::Value]);
    }
}
