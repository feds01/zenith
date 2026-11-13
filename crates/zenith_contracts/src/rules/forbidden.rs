//! The `forbidden` contract type: the `from` units must not import the `to`
//! units.

use rayon::prelude::*;
use zenith_config::ForbiddenConfig;
use zenith_graph::{GraphView, NodeSet, Unit};

use crate::{
    Chain, Violation,
    selection::{ImportedUnit, UnitResolver},
    view::Statements,
};

/// A compiled `forbidden` contract.
#[derive(Debug)]
pub(crate) struct Forbidden {
    /// The units that must not import the `to` units.
    from: Vec<Unit>,

    /// The units that must not be imported, with the names that an import of
    /// each must take to count.
    to: Vec<ImportedUnit>,

    /// Whether only direct imports count, rather than chains of imports.
    allow_indirect: bool,
}

impl Forbidden {
    /// Compile `config`, resolving its selectors with `units`.
    pub(crate) fn compile(config: &ForbiddenConfig, units: &mut UnitResolver) -> Self {
        Self {
            from: units.units(&config.from),
            to: units.imported_units(&config.to),
            allow_indirect: config.allow_indirect_imports,
        }
    }

    /// The chains from each `from` unit to each `to` unit in `view`, one
    /// violation per pair of units, in the order of the units.
    pub(crate) fn check(&self, view: &GraphView, statements: &Statements) -> Vec<Violation> {
        // 1. See each `to` unit with `names` through the imports of it that take one of
        //    them, in a view of its own, so that a chain to another unit may still pass
        //    through it. The others, and every unit of a contract without `names`, are
        //    seen in `view`.
        let narrowed: Vec<Option<GraphView>> = match self.to.iter().any(|to| to.names.is_some()) {
            true => (self.to.iter())
                .map(|to| to.names.is_some().then(|| Self::importing(view, statements, to)))
                .collect(),
            false => Vec::new(),
        };
        let view_of = |index: usize| narrowed.get(index).and_then(Option::as_ref).unwrap_or(view);

        // 2. Find everything that reaches each `to` unit, so that a pair without a
        //    chain is ruled out without a search.
        let reachers: Vec<NodeSet> = if self.allow_indirect {
            Vec::new()
        } else {
            (self.to.par_iter().enumerate())
                .map(|(index, to)| view_of(index).find_downstream(&to.unit.members))
                .collect()
        };

        // 3. Search every pair of units, in parallel.
        let pairs: Vec<(&Unit, usize)> = self
            .from
            .iter()
            .flat_map(|from| (0..self.to.len()).map(move |to| (from, to)))
            .collect();
        pairs
            .into_par_iter()
            .filter_map(|(from, to_index)| {
                let (imported, view) = (&self.to[to_index], view_of(to_index));
                let to = &imported.unit;
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
                let chains = paths.iter().map(|path| {
                    // The import of the unit shows the statements that take its names.
                    let mut chain = Chain::from_path(view, statements, path);
                    if imported.names.is_some() {
                        for link in &mut chain.links {
                            if to.members.contains(link.imported) {
                                link.imports.retain(|detail| imported.takes(detail));
                            }
                        }
                    }
                    chain
                });
                Some(Violation::Dependency {
                    importer: from.label.clone(),
                    imported: to.label.clone(),
                    chains: chains.collect(),
                })
            })
            .collect()
    }

    /// `view`, without the imports of `to` none of whose statements that count
    /// take one of its names.
    fn importing<'g>(
        view: &GraphView<'g>,
        statements: &Statements,
        to: &ImportedUnit,
    ) -> GraphView<'g> {
        let graph = view.graph();
        let mut untaken = Vec::new();
        for member in to.unit.members.iter() {
            for importer in view.importers_of(member) {
                let details = graph.details(importer, member);
                if !details
                    .iter()
                    .any(|detail| statements.count(importer, detail) && to.takes(detail))
                {
                    untaken.push((importer, member));
                }
            }
        }
        view.without(untaken)
    }
}

#[cfg(test)]
mod tests {
    use zenith_graph::ImportKind;

    use crate::{
        ContractErrorKind,
        testing::{Project, values},
    };

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
    fn chains_do_not_pass_through_boundaries() {
        let project = Project::with_directives(
            &values(&[
                // A server action, which the client imports by reference.
                ("web/upload.tsx", "web/actions.ts"),
                ("web/actions.ts", "node:fs"),
                ("web/actions.ts", "web/lib.ts"),
                ("web/lib.ts", "npm:sharp"),
                // The same module, imported directly.
                ("web/preview.tsx", "web/lib.ts"),
            ]),
            &[
                ("web/upload.tsx", "use client"),
                ("web/preview.tsx", "use client"),
                ("web/actions.ts", "use server"),
            ],
            include_str!("fixtures/forbidden-boundaries.toml"),
        );
        // A chain may still end at a boundary.
        assert_eq!(
            project.findings("f"),
            [
                "web/preview.tsx -> npm:sharp: web/preview.tsx -> web/lib.ts -> npm:sharp",
                "web/upload.tsx -> web/actions.ts: web/upload.tsx -> web/actions.ts",
            ]
        );
    }

    #[test]
    fn chains_may_start_at_a_boundary() {
        let project = Project::new(
            &values(&[("web/actions.ts", "lib/files.ts"), ("lib/files.ts", "node:fs")]),
            &[],
            include_str!("fixtures/forbidden-boundary-start.toml"),
        );
        assert_eq!(
            project.findings("f"),
            ["web -> node:fs: web/actions.ts -> lib/files.ts -> node:fs"]
        );
    }

    #[test]
    fn a_boundary_that_matches_nothing_is_an_error() {
        let project = Project::new(
            &values(&[("web/a.ts", "node:fs")]),
            &[],
            include_str!("fixtures/forbidden-unmatched-boundary.toml"),
        );
        let errors = project.compile().unwrap_err();
        let kinds: Vec<_> = errors.iter().map(|error| &error.kind).collect();
        let expected = ContractErrorKind::UnmatchedSelector { selector: "web/nope".to_string() };
        assert_eq!(kinds, [&expected]);
    }

    #[test]
    fn names_keep_the_imports_that_take_one_of_them_as_a_value() {
        let project = Project::with_names(
            &[
                ("web/hooks.ts", "npm:react", "forwardRef useLayoutEffect"),
                ("web/button.tsx", "npm:react", "forwardRef"),
                ("web/legacy.tsx", "npm:react", "default"),
                // The whole module, which a namespace or side-effect import takes, and
                // a name whose type only it takes.
                ("web/everything.ts", "npm:react", "*"),
                ("web/props.ts", "npm:react", "useLayoutEffect:type"),
                ("web/guard.ts", "npm:client-only", "*"),
            ],
            include_str!("fixtures/forbidden-names.toml"),
        );
        assert_eq!(
            project.findings("f"),
            [
                "web -> npm:react: web/hooks.ts -> npm:react",
                "web -> npm:react: web/legacy.tsx -> npm:react",
                "web -> npm:client-only: web/guard.ts -> npm:client-only",
            ]
        );
    }

    #[test]
    fn a_link_into_a_unit_with_names_shows_the_statements_that_take_them() {
        let project = Project::with_names(
            &[
                ("web/page.tsx", "lib/hooks.ts", "useThing"),
                ("web/page.tsx", "npm:react", "forwardRef"),
                ("web/page.tsx", "npm:react", "useLayoutEffect"),
            ],
            include_str!("fixtures/forbidden-names.toml"),
        );
        let report = project.check("f");
        let crate::Violation::Dependency { chains, .. } = &report.violations[0] else {
            panic!("expected a dependency");
        };
        let starts: Vec<_> =
            chains[0].links[0].imports.iter().map(|detail| detail.range.start()).collect();
        assert_eq!(starts, [300]);
    }

    #[test]
    fn chains_pass_through_a_unit_whose_names_they_do_not_take() {
        let project = Project::with_names(
            &[("web/page.tsx", "lib/hooks.ts", "Button"), ("lib/hooks.ts", "npm:client-only", "*")],
            include_str!("fixtures/forbidden-names-through.toml"),
        );
        // `web/page.tsx` imports the hooks, but not `useThing`.
        assert_eq!(
            project.findings("f"),
            ["web -> npm:client-only: web/page.tsx -> lib/hooks.ts -> npm:client-only"]
        );
    }

    #[test]
    fn a_unit_that_a_selector_without_names_matches_counts_every_import() {
        let project = Project::with_names(
            &[("web/a.ts", "npm:react", "useEffect"), ("web/b.ts", "npm:lodash", "useState")],
            include_str!("fixtures/forbidden-names-merged.toml"),
        );
        assert_eq!(
            project.findings("f"),
            [
                "web -> npm:lodash: web/b.ts -> npm:lodash",
                "web -> npm:react: web/a.ts -> npm:react"
            ]
        );
    }

    #[test]
    fn a_unit_that_several_selectors_with_names_match_counts_the_names_of_each() {
        let project = Project::with_names(
            &[
                ("web/a.ts", "npm:react", "useState"),
                ("web/b.ts", "npm:react", "useEffect"),
                ("web/c.ts", "npm:react", "forwardRef"),
            ],
            include_str!("fixtures/forbidden-names-both.toml"),
        );
        assert_eq!(
            project.findings("f"),
            ["web -> npm:react: web/a.ts -> npm:react", "web -> npm:react: web/b.ts -> npm:react"]
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
