//! Contracts compiled against a project, and checking them.

use std::time::{Duration, Instant};

use rayon::prelude::*;
use zenith_config::{AlertLevel, ContractConfig, IgnoreImport, InContract, Selector};
use zenith_graph::{Graph, ModuleId, NodeSet, Unit};
use zenith_lang::Workspace;
use zenith_pipeline::{FileSuppressions, Graphs, Scope};
use zenith_utils::location::ByteRange;

use crate::{
    Baseline, ContractError, ContractErrorKind, Selection, Violation, rules::Rule,
    view::ContractView,
};

/// A contract of the configuration, compiled against a project: its
/// selectors are resolved into units.
#[derive(Debug)]
pub struct Contract<'c> {
    /// The contract, as configured.
    pub config: &'c ContractConfig,

    /// What the contract checks, with its units.
    rule: Rule,

    /// The members of its boundaries, which chains may start or end at, but
    /// not pass through.
    boundaries: NodeSet,
}

impl<'c> Contract<'c> {
    /// Compile `config`, resolving its selectors against the project's
    /// `graph` and the `workspace` packages that `pkg:` selectors match.
    ///
    /// A path or `pkg:` selector that matches nothing is an error, as it is
    /// likely a typo or a rename. Every error in the contract is returned.
    pub fn compile(
        config: &'c ContractConfig,
        graph: &Graph,
        workspace: &Workspace,
    ) -> Result<Self, Vec<ContractError>> {
        let mut units = UnitResolver { contract: &config.id, graph, workspace, errors: Vec::new() };
        let rule = Rule::compile(&config.kind, &mut units);
        let boundaries = units.members(config.kind.boundaries());
        if units.errors.is_empty() {
            Ok(Self { config, rule, boundaries })
        } else {
            Err(units.errors)
        }
    }

    /// Compile every contract in `configs`, as [`Contract::compile`] does.
    /// If any contract has errors, every error of every contract is returned,
    /// in the order of the contracts.
    pub fn compile_all(
        configs: impl IntoIterator<Item = &'c ContractConfig>,
        graph: &Graph,
        workspace: &Workspace,
    ) -> Result<Vec<Self>, Vec<ContractError>> {
        let mut contracts = Vec::new();
        let mut errors = Vec::new();
        for config in configs {
            match Self::compile(config, graph, workspace) {
                Ok(contract) => contracts.push(contract),
                Err(contract_errors) => errors.extend(contract_errors),
            }
        }
        if errors.is_empty() { Ok(contracts) } else { Err(errors) }
    }

    /// Check the contract against `graph`, without the imports that the
    /// contract leaves out: the statements of its excluded kinds, those that
    /// the project `exempts` from it, and the imports its `ignore_imports`
    /// match. Chains don't pass through its boundaries.
    pub fn check(
        &self,
        graph: &Graph,
        workspace: &Workspace,
        exempts: &Exemptions,
    ) -> ContractReport<'c> {
        self.check_leaving_out(graph, workspace, exempts, &[])
    }

    /// [`Contract::check`], also without the `left_out` imports, which count
    /// as baselined.
    pub(crate) fn check_leaving_out(
        &self,
        graph: &Graph,
        workspace: &Workspace,
        exempts: &Exemptions,
        left_out: &[(ModuleId, ModuleId)],
    ) -> ContractReport<'c> {
        let start = Instant::now();
        let mut view =
            ContractView::new(self.config, graph, workspace, exempts, left_out, &self.boundaries);
        let violations = self.rule.check(&view.view, &view.statements);
        view.used_suppressions.sort_unstable();
        view.used_suppressions.dedup();
        ContractReport {
            config: self.config,
            violations,
            unmatched_ignores: view.unmatched,
            ignored: view.ignored,
            suppressed: view.suppressed,
            baselined: view.baselined,
            stale_entries: view.stale_entries,
            used_suppressions: view.used_suppressions,
            duration: start.elapsed(),
        }
    }

    /// Check `contracts` in parallel, as [`Contract::check`] does, each
    /// against the graph of `graphs` that it sees. The reports are in the
    /// order of the contracts.
    pub fn check_all(
        contracts: &[Self],
        graphs: Graphs,
        workspace: &Workspace,
        exempts: &Exemptions,
    ) -> Vec<ContractReport<'c>> {
        contracts
            .par_iter()
            .map(|contract| contract.check(graphs.of(contract.config), workspace, exempts))
            .collect()
    }
}

/// What the project exempts from its contracts, besides the imports that
/// each contract ignores itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct Exemptions<'a> {
    /// The `zenith-ignore` comments of the files that have any, sorted by
    /// file. Each leaves the import statements that start on its target line
    /// out of the contracts it names.
    pub suppressions: &'a [FileSuppressions],

    /// The baseline, whose entries for each contract are left out of it, or
    /// `None` to ignore it.
    pub baseline: Option<&'a Baseline>,
}

/// The result of checking a contract.
#[derive(Debug)]
pub struct ContractReport<'c> {
    /// The contract, as configured.
    pub config: &'c ContractConfig,

    /// How the contract is broken, in a deterministic order.
    pub violations: Vec<Violation>,

    /// The `ignore_imports` entries that match no import, in the order of the
    /// configuration. Whether they break the contract depends on its
    /// `unmatched_ignore_imports`.
    pub unmatched_ignores: Vec<&'c IgnoreImport>,

    /// The number of imports that its `ignore_imports` left out.
    pub ignored: usize,

    /// The number of import statements that suppressions left out.
    pub suppressed: usize,

    /// The number of imports that the baseline left out.
    pub baselined: usize,

    /// The baseline entries of the contract whose import no longer exists, in
    /// the order of the baseline.
    pub stale_entries: Vec<String>,

    /// The suppressions that left out a statement, sorted, as the index of
    /// their file in the exempted suppressions and their index in it.
    pub(crate) used_suppressions: Vec<(usize, usize)>,

    /// How long checking the contract took.
    pub duration: Duration,
}

impl ContractReport<'_> {
    /// Keep only the findings that involve a file in `scope`: the violations
    /// with an import from or into one, as [`Violation::narrow`] keeps them,
    /// and the stale baseline entries of one. `graph` is the graph the
    /// contract was checked against. The unmatched `ignore_imports` entries
    /// are about the configuration, not a file, so they are all kept.
    pub fn narrow(&mut self, graph: &Graph, scope: &Scope) {
        if scope.is_everything() {
            return;
        }
        self.violations.retain_mut(|violation| violation.narrow(graph, scope));
        self.stale_entries.retain(|entry| entry.split(" -> ").any(|name| scope.contains(name)));
    }

    /// The `ignore_imports` entries that match no import and break the
    /// contract, because its `unmatched_ignore_imports` is `error`.
    pub fn unmatched_ignore_errors(&self) -> &[&IgnoreImport] {
        match self.config.unmatched_ignore_imports {
            AlertLevel::Error => &self.unmatched_ignores,
            AlertLevel::Warn | AlertLevel::None => &[],
        }
    }

    /// Whether the contract is kept: nothing violates it, and no
    /// `ignore_imports` entry that must match something matches nothing.
    pub fn is_kept(&self) -> bool {
        self.violations.is_empty() && self.unmatched_ignore_errors().is_empty()
    }

    /// The number of findings that break the contract: the chains and
    /// imports of its violations, and its unmatched `ignore_imports` entries
    /// that are errors.
    pub fn finding_count(&self) -> usize {
        let violations: usize = self.violations.iter().map(Violation::finding_count).sum();
        violations + self.unmatched_ignore_errors().len()
    }
}

/// Resolves the selectors of one contract into units, collecting the errors.
pub(crate) struct UnitResolver<'a> {
    /// The id of the contract.
    contract: &'a str,

    /// The project's graph.
    graph: &'a Graph,

    /// The project's workspace packages.
    workspace: &'a Workspace,

    /// The errors so far.
    errors: Vec<ContractError>,
}

impl<'a> UnitResolver<'a> {
    /// The project's graph.
    pub(crate) fn graph(&self) -> &'a Graph {
        self.graph
    }

    /// The project's workspace packages.
    pub(crate) fn workspace(&self) -> &'a Workspace {
        self.workspace
    }

    /// The units of `selectors`, in order, each unit once even if several
    /// selectors match it. A path or `pkg:` selector that matches nothing is
    /// an error.
    pub(crate) fn units(&mut self, selectors: &[Selector]) -> Vec<Unit> {
        self.located_units(selectors).into_iter().map(|(unit, _)| unit).collect()
    }

    /// The members of every unit of `selectors`, as [`Self::units`] finds
    /// them.
    pub(crate) fn members(&mut self, selectors: &[Selector]) -> NodeSet {
        let mut members = NodeSet::new();
        for unit in self.units(selectors) {
            members.union_with(&unit.members);
        }
        members
    }

    /// The units of `selectors`, as [`Self::units`] finds them, each with the
    /// range of the selector that matched it.
    pub(crate) fn located_units(
        &mut self,
        selectors: &[Selector],
    ) -> Vec<(Unit, Option<ByteRange>)> {
        let mut units: Vec<(Unit, Option<ByteRange>)> = Vec::new();
        for selector in selectors {
            match Selection::try_resolve(selector, self.graph, self.workspace) {
                Ok(selection) => {
                    for unit in selection.units {
                        if !units.iter().any(|(existing, _)| existing.root == unit.root) {
                            units.push((unit, selector.range()));
                        }
                    }
                }
                Err(unmatched) => self.error(
                    ContractErrorKind::UnmatchedSelector { selector: unmatched.selector },
                    selector.range(),
                ),
            }
        }
        units
    }

    /// Report the first pair of `units` that overlap, at the range of the
    /// later one.
    pub(crate) fn ensure_apart(&mut self, units: &[(Unit, Option<ByteRange>)]) {
        for (index, (first, _)) in units.iter().enumerate() {
            let overlapping = units[index + 1..]
                .iter()
                .find(|(second, _)| first.members.intersects(&second.members));
            if let Some((second, range)) = overlapping {
                let kind = ContractErrorKind::OverlappingModules {
                    first: first.label.clone(),
                    second: second.label.clone(),
                };
                self.error(kind, *range);
                return;
            }
        }
    }

    /// Record an error in the contract, at `range` in the configuration file.
    pub(crate) fn error(&mut self, kind: ContractErrorKind, range: Option<ByteRange>) {
        let contract = InContract(self.contract.to_string());
        self.errors.push(ContractError { contract, kind, range });
    }
}

#[cfg(test)]
mod tests {
    use zenith_config::{AlertLevel, Severity};
    use zenith_pipeline::Scope;

    use crate::{
        Violation,
        testing::{Project, values},
    };

    #[test]
    fn narrowing_keeps_the_findings_that_involve_the_scope() {
        let imports = values(&[
            ("web/a.ts", "api/x.ts"),
            ("web/b.ts", "api/y.ts"),
            ("m/a/x.ts", "m/b/y.ts"),
            ("m/b/y.ts", "m/a/x.ts"),
            ("domains/orders/helpers/a.ts", "domains/orders/types.ts"),
        ]);
        let project = Project::new(&imports, &[], include_str!("fixtures/narrow.toml"))
            .baseline("f", &["web/a.ts -> api/z.ts", "web/old.ts -> api/x.ts"]);
        let narrowed = |paths: &[&str]| {
            let scope = Scope::new(paths.iter().map(|path| path.to_string()).collect());
            let mut found = Vec::new();
            for id in ["f", "c", "l", "n"] {
                let mut report = project.check(id);
                report.narrow(&project.graph, &scope);
                found.extend(report.violations.iter().flat_map(|v| project.describe(v)));
                found.extend(report.stale_entries.iter().map(|entry| format!("stale {entry}")));
            }
            found
        };

        // An importing file, a directory with a file of a cycle breaker in it,
        // and a file inside an undeclared module. A unit over its limit keeps every
        // import that makes up its count.
        assert_eq!(
            narrowed(&["web/a.ts", "m/a", "domains/orders/helpers/a.ts"]),
            [
                "web -> api: web/a.ts -> api/x.ts",
                "stale web/a.ts -> api/z.ts",
                "m/*: break m/b -> m/a (m/b/y.ts -> m/a/x.ts) in m/b -> m/a",
                "domains/orders: `domains/orders/helpers` is not a layer",
                "api: FanIn 2 > 1: web/a.ts -> api/x.ts, web/b.ts -> api/y.ts",
            ]
        );
        // An imported file.
        assert_eq!(
            narrowed(&["api/y.ts"]),
            [
                "web -> api: web/b.ts -> api/y.ts",
                "api: FanIn 2 > 1: web/a.ts -> api/x.ts, web/b.ts -> api/y.ts",
            ]
        );
        // A directory in none of the imports of the unit over its limit.
        assert_eq!(
            narrowed(&["m/b"]),
            ["m/*: break m/b -> m/a (m/b/y.ts -> m/a/x.ts) in m/b -> m/a"]
        );
        // Nothing, and everything.
        assert!(narrowed(&["other"]).is_empty());
        assert_eq!(narrowed(&[]).len(), 7);
    }

    #[test]
    fn every_unmatched_selector_is_an_error_at_its_place_in_the_file() {
        let toml = "[contracts.f]
type = \"forbidden\"
from = [\"web\", \"wbe\"]
to = [\"npm:sharp\", \"pkg:missing\"]

[contracts.p]
type = \"protected\"
modules = [\"web/lib\"]
allowed_importers = [\"web/aap\"]
";
        let project = Project::new(&values(&[("web/lib/a.ts", "npm:sharp")]), &[], toml);
        let errors = project.compile().unwrap_err();
        let found: Vec<_> = errors
            .iter()
            .map(|error| {
                let range = error.range.unwrap();
                (
                    error.contract.0.as_str(),
                    error.kind.to_string(),
                    &toml[range.start() as usize..range.end() as usize],
                )
            })
            .collect();
        // `npm:sharp` may match nothing; it would only mean nothing imports it.
        assert_eq!(
            found,
            [
                ("f", "`wbe` does not match anything".to_string(), "\"wbe\""),
                ("f", "`pkg:missing` does not match anything".to_string(), "\"pkg:missing\""),
                ("p", "`web/aap` does not match anything".to_string(), "\"web/aap\""),
            ]
        );
    }

    #[test]
    fn ignored_imports_are_left_out_of_the_view() {
        let toml = "[contracts.f]
type = \"forbidden\"
from = [\"a\"]
to = [\"b\"]
ignore_imports = [\"a/x.ts -> b/y.ts\"]
";
        let project = Project::new(
            &values(&[("a/x.ts", "b/y.ts"), ("a/w.ts", "c/z.ts"), ("c/z.ts", "b/y.ts")]),
            &[],
            toml,
        );
        // The ignored import is gone, but the chain through `c` remains.
        assert_eq!(project.findings("f"), ["a -> b: a/w.ts -> c/z.ts -> b/y.ts"]);
        assert!(project.check("f").unmatched_ignores.is_empty());
    }

    #[test]
    fn unmatched_ignored_imports_break_the_contract_unless_relaxed() {
        let contract = |level: &str| {
            format!(
                "[contracts.f]\ntype = \"forbidden\"\nfrom = [\"a\"]\nto = [\"b\"]\nignore_imports = [\"a -> c\"]\nunmatched_ignore_imports = \"{level}\"\n"
            )
        };
        let imports = values(&[("b/y.ts", "a/x.ts"), ("c/z.ts", "a/x.ts")]);
        for (level, kept, findings) in [("error", false, 1), ("warn", true, 0), ("none", true, 0)] {
            let project = Project::new(&imports, &[], &contract(level));
            let report = project.check("f");
            let unmatched: Vec<_> =
                report.unmatched_ignores.iter().map(|entry| entry.as_str()).collect();
            assert_eq!(unmatched, ["a -> c"], "{level}");
            assert_eq!(report.is_kept(), kept, "{level}");
            assert_eq!(report.finding_count(), findings, "{level}");
        }
        let project = Project::new(&imports, &[], &contract("warn"));
        assert_eq!(project.config.contracts[0].unmatched_ignore_imports, AlertLevel::Warn);
    }

    #[test]
    fn suppressed_statements_are_left_out_of_the_contracts_they_name() {
        let toml = "[contracts.f]
type = \"forbidden\"
from = [\"a\"]
to = [\"b\"]

[contracts.g]
type = \"forbidden\"
from = [\"a\"]
to = [\"b\"]
ignore_imports = [\"a/w.ts -> b/z.ts\"]
";
        let imports = values(&[
            ("a/x.ts", "b/y.ts"),
            ("a/x.ts", "b/y.ts"),
            ("a/w.ts", "b/z.ts"),
            ("a/v.ts", "b/z.ts"),
        ]);
        let project = Project::new(&imports, &[], toml)
            .suppress("a/x.ts", 1, "f")
            .suppress("a/w.ts", 3, "f g")
            .suppress("a/v.ts", 4, "g");

        // The second statement of `a/x.ts` keeps its import in `f`, but the
        // link only shows that statement. `a/v.ts` is only suppressed in `g`.
        let report = project.check("f");
        assert_eq!(project.findings("f"), ["a -> b: a/v.ts -> b/z.ts", "a -> b: a/x.ts -> b/y.ts"]);
        let Violation::Dependency { chains, .. } = &report.violations[0] else {
            panic!("expected a dependency, got {:?}", report.violations[0]);
        };
        let starts: Vec<_> =
            chains[1].links[0].imports.iter().map(|import| import.range.start()).collect();
        assert_eq!(starts, [200]);
        assert_eq!((report.suppressed, report.ignored), (2, 0));

        // In `g`, the import that is both suppressed and ignored counts once,
        // as suppressed.
        let report = project.check("g");
        assert_eq!(project.findings("g"), ["a -> b: a/x.ts -> b/y.ts"]);
        assert_eq!((report.suppressed, report.ignored), (2, 0));
        assert!(report.unmatched_ignores.is_empty());
    }

    #[test]
    fn ignored_imports_are_counted_once() {
        let toml = "[contracts.f]
type = \"forbidden\"
from = [\"a\"]
to = [\"b\"]
ignore_imports = [\"a -> b\", \"a/x.ts -> b/y.ts\"]
";
        let project =
            Project::new(&values(&[("a/x.ts", "b/y.ts"), ("a/w.ts", "b/y.ts")]), &[], toml);
        let report = project.check("f");
        assert!(report.is_kept());
        assert_eq!((report.ignored, report.suppressed), (2, 0));
    }

    #[test]
    fn contracts_inherit_the_excluded_kinds_of_the_analysis() {
        let toml = "[analysis]
exclude_kinds = [\"type\"]

[contracts.inherits]
type = \"forbidden\"
from = [\"a\"]
to = [\"b\"]

[contracts.overrides]
type = \"forbidden\"
from = [\"a\"]
to = [\"b\"]
exclude_kinds = []
";
        let imports = [("a/x.ts", "b/y.ts", zenith_graph::ImportKind::Type)];
        let project = Project::new(&imports, &[], toml);
        assert!(project.check("inherits").is_kept());
        assert_eq!(project.findings("overrides"), ["a -> b: a/x.ts -> b/y.ts"]);
    }

    #[test]
    fn contracts_are_checked_in_parallel_and_reported_in_order() {
        let mut toml = String::new();
        for id in ["c", "a", "b"] {
            toml += &format!(
                "[contracts.{id}]\ntype = \"forbidden\"\nfrom = [\"x\"]\nto = [\"y\"]\nseverity = \"warn\"\n"
            );
        }
        let project = Project::new(&values(&[("x/a.ts", "y/b.ts")]), &[], &toml);
        let contracts = project.compile().unwrap();
        let exemptions = project.exemptions();
        let reports = crate::Contract::check_all(
            &contracts,
            (&project.graph).into(),
            &project.workspace,
            &exemptions,
        );
        let ids: Vec<_> = reports.iter().map(|report| report.config.id.as_str()).collect();
        assert_eq!(ids, ["c", "a", "b"]);
        assert!(reports.iter().all(|report| !report.is_kept() && report.finding_count() == 1));
        assert!(reports.iter().all(|report| report.config.severity == Severity::Warn));
    }
}
