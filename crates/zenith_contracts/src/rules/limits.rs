//! The `limits` contract type: each module may only be imported by so many
//! other nodes, and import so many.

use zenith_config::LimitsConfig;
use zenith_graph::{GraphView, Unit};

use crate::{Limit, Link, Violation, contract::UnitResolver, view::Statements};

/// A compiled `limits` contract.
#[derive(Debug)]
pub(crate) struct Limits {
    /// The limited units.
    modules: Vec<Unit>,

    /// Each limit that is set, with the most that it allows.
    limits: Vec<(Limit, usize)>,
}

impl Limits {
    /// Compile `config`, resolving its selectors with `units`.
    pub(crate) fn compile(config: &LimitsConfig, units: &mut UnitResolver) -> Self {
        let limits = [(Limit::FanIn, config.max_fan_in), (Limit::FanOut, config.max_fan_out)];
        Self {
            modules: units.units(&config.modules),
            limits: limits.into_iter().filter_map(|(limit, max)| Some((limit, max?))).collect(),
        }
    }

    /// The units that count more distinct nodes outside them than a limit
    /// allows: the files that import a unit for its fan-in, and the files
    /// and packages of the project that it imports for its fan-out. Imports
    /// between its own members, and of external packages and builtins, don't
    /// count. One violation per unit and limit, in the order of the units,
    /// fan-in first.
    pub(crate) fn check(&self, view: &GraphView, statements: &Statements) -> Vec<Violation> {
        let mut violations = Vec::new();
        for unit in &self.modules {
            for &(limit, max) in &self.limits {
                let imports = Self::imports(view, statements, unit, limit);
                let count = limit.groups(&imports).count();
                if count > max {
                    violations.push(Violation::LimitExceeded {
                        module: unit.label.clone(),
                        limit,
                        count,
                        max,
                        imports,
                    });
                }
            }
        }
        violations
    }

    /// The direct imports between `unit` and the nodes outside it that
    /// `limit` counts, in the order of the counted node, then of the other.
    fn imports(view: &GraphView, statements: &Statements, unit: &Unit, limit: Limit) -> Vec<Link> {
        let mut imports = Vec::new();
        for member in unit.members.iter() {
            match limit {
                Limit::FanIn => {
                    for importer in view.importers_of(member) {
                        if !unit.members.contains(importer) {
                            imports.push(Link::new(view, statements, importer, member));
                        }
                    }
                }
                Limit::FanOut => {
                    for imported in view.imports_of(member) {
                        let internal = view.graph().node(imported).is_internal();
                        if internal && !unit.members.contains(imported) {
                            imports.push(Link::new(view, statements, member, imported));
                        }
                    }
                }
            }
        }
        imports.sort_by_key(|link| (limit.counted(link), link.importer, link.imported));
        imports
    }
}

#[cfg(test)]
mod tests {
    use zenith_graph::ImportKind;

    use crate::testing::{Project, values};

    #[test]
    fn units_over_a_limit_are_violations() {
        let project = Project::new(
            &values(&[
                // `web/lib` is imported by three files, one of them twice: its fan-in is 3.
                ("web/app/a.ts", "web/lib/x.ts"),
                ("web/app/a.ts", "web/lib/y.ts"),
                ("web/app/b.ts", "web/lib/x.ts"),
                ("web/ui/c.ts", "web/lib/y.ts"),
                // Imports between its members don't count.
                ("web/lib/x.ts", "web/lib/y.ts"),
                // `web/app` imports `web/lib`'s two files and `web/ui/c.ts`: its fan-out
                // is 3. External packages and builtins don't count.
                ("web/app/b.ts", "web/ui/c.ts"),
                ("web/app/b.ts", "npm:react"),
                ("web/app/a.ts", "npm:react"),
                ("web/app/a.ts", "node:fs"),
                // `web/ui` is within both limits.
            ]),
            &[],
            include_str!("fixtures/limits.toml"),
        );
        assert_eq!(
            project.findings("l"),
            [
                "web/app: FanOut 3 > 2: web/app/a.ts -> web/lib/x.ts, \
                 web/app/b.ts -> web/lib/x.ts, web/app/a.ts -> web/lib/y.ts, \
                 web/app/b.ts -> web/ui/c.ts",
                "web/lib: FanIn 3 > 2: web/app/a.ts -> web/lib/x.ts, web/app/a.ts -> web/lib/y.ts, \
                 web/app/b.ts -> web/lib/x.ts, web/ui/c.ts -> web/lib/y.ts",
            ]
        );
    }

    #[test]
    fn a_package_imported_as_a_whole_counts_once() {
        let project = Project::new(
            &values(&[
                // `cmd` imports the package `internal/db` from two files, and the
                // package `internal/cache`: its fan-out is 2.
                ("cmd/a.go", "internal/db/"),
                ("cmd/b.go", "internal/db/"),
                ("cmd/a.go", "internal/cache/"),
                // `internal/db` is imported by `cmd`'s two files, and by a file of
                // `internal/cache`: its fan-in is 3. Its own file's import doesn't count.
                ("internal/cache/store.go", "internal/db/"),
                ("internal/db/conn.go", "internal/db/"),
            ]),
            &[],
            include_str!("fixtures/limits-packages.toml"),
        );
        assert_eq!(
            project.findings("l"),
            [
                "cmd: FanOut 2 > 1: cmd/a.go -> internal/cache, cmd/a.go -> internal/db, \
                 cmd/b.go -> internal/db",
                "internal/db: FanIn 3 > 2: cmd/a.go -> internal/db, cmd/b.go -> internal/db, \
                 internal/cache/store.go -> internal/db",
            ]
        );
    }

    #[test]
    fn external_packages_have_a_fan_in() {
        let project = Project::new(
            &values(&[
                ("web/a.ts", "npm:react"),
                ("web/b.ts", "npm:react"),
                ("web/a.ts", "npm:zod"),
            ]),
            &[],
            include_str!("fixtures/limits-externals.toml"),
        );
        assert_eq!(
            project.findings("l"),
            ["npm:react: FanIn 2 > 1: web/a.ts -> npm:react, web/b.ts -> npm:react"]
        );
    }

    #[test]
    fn a_unit_at_its_limit_is_kept() {
        let project = Project::new(
            &values(&[
                ("web/app/a.ts", "web/lib/x.ts"),
                ("web/app/b.ts", "web/lib/x.ts"),
                ("web/app/a.ts", "web/ui/c.ts"),
            ]),
            &[],
            include_str!("fixtures/limits.toml"),
        );
        assert!(project.check("l").is_kept());
    }

    #[test]
    fn the_excluded_kinds_and_ignored_imports_do_not_count() {
        let project = Project::new(
            &[
                ("web/app/a.ts", "web/lib/x.ts", ImportKind::Value),
                ("web/app/b.ts", "web/lib/x.ts", ImportKind::Value),
                ("web/app/c.ts", "web/lib/x.ts", ImportKind::Type),
                ("web/ui/d.ts", "web/lib/x.ts", ImportKind::Value),
            ],
            &[],
            include_str!("fixtures/limits-exemptions.toml"),
        );
        assert!(project.check("l").is_kept());
        assert!(!project.check("all").is_kept());
    }

    #[test]
    fn the_baseline_records_the_imports_over_the_limit() {
        let project = Project::new(
            &values(&[
                ("web/app/a.ts", "web/lib/x.ts"),
                ("web/app/b.ts", "web/lib/x.ts"),
                ("web/app/b.ts", "web/lib/y.ts"),
                ("web/ui/c.ts", "web/lib/y.ts"),
                ("web/ui/d.ts", "web/lib/x.ts"),
            ]),
            &[],
            include_str!("fixtures/limits-fan-in.toml"),
        );
        // The importers over the limit of 2 are the last ones, with every import of
        // each.
        let entries = project.baseline_entries("l");
        assert_eq!(entries, ["web/ui/c.ts -> web/lib/y.ts", "web/ui/d.ts -> web/lib/x.ts"]);
        let entries: Vec<_> = entries.iter().map(String::as_str).collect();
        assert!(project.baseline("l", &entries).check("l").is_kept());
    }
}
