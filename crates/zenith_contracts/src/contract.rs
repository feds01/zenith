//! Contracts compiled against a project, and checking them.

use std::time::{Duration, Instant};

use rayon::prelude::*;
use zenith_config::{AlertLevel, ContractConfig, IgnoreImport, Selector};
use zenith_graph::{Graph, Unit};
use zenith_utils::location::ByteRange;
use zenith_workspace::Workspace;

use crate::{
    ContractError, ContractErrorKind, Selection, Violation, rules::Rule, view::ContractView,
};

/// A contract of the configuration, compiled against a project: its
/// selectors are resolved into units.
#[derive(Debug)]
pub struct Contract<'c> {
    /// The contract, as configured.
    pub config: &'c ContractConfig,

    /// What the contract checks, with its units.
    rule: Rule,
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
        match rule {
            Some(rule) if units.errors.is_empty() => Ok(Self { config, rule }),
            _ => Err(units.errors),
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
    /// contract leaves out: those only made by statements of its excluded
    /// kinds, and those its `ignore_imports` match.
    pub fn check(&self, graph: &Graph, workspace: &Workspace) -> ContractReport<'c> {
        let start = Instant::now();
        let ContractView { view, unmatched } = ContractView::new(self.config, graph, workspace);
        let violations = self.rule.check(&view, self.config.exclude_kinds);
        ContractReport {
            config: self.config,
            violations,
            unmatched_ignores: unmatched,
            duration: start.elapsed(),
        }
    }

    /// Check `contracts` in parallel, as [`Contract::check`] does. The
    /// reports are in the order of the contracts.
    pub fn check_all(
        contracts: &[Self],
        graph: &Graph,
        workspace: &Workspace,
    ) -> Vec<ContractReport<'c>> {
        contracts.par_iter().map(|contract| contract.check(graph, workspace)).collect()
    }
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

    /// How long checking the contract took.
    pub duration: Duration,
}

impl ContractReport<'_> {
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
        self.errors.push(ContractError { contract: self.contract.to_string(), kind, range });
    }
}

#[cfg(test)]
mod tests {
    use zenith_config::{AlertLevel, Severity};

    use crate::{
        ContractErrorKind,
        testing::{Project, values},
    };

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
                    error.contract.as_str(),
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
    fn contract_types_that_are_not_supported_yet_are_errors() {
        let project = Project::new(
            &values(&[("a/x.ts", "b/y.ts")]),
            &[],
            "[contracts.c]\ntype = \"acyclic\"\nmodules = [\"a\", \"b\"]\n",
        );
        let errors = project.compile().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ContractErrorKind::Unsupported { kind: "acyclic" });
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
        let reports = crate::Contract::check_all(&contracts, &project.graph, &project.workspace);
        let ids: Vec<_> = reports.iter().map(|report| report.config.id.as_str()).collect();
        assert_eq!(ids, ["c", "a", "b"]);
        assert!(reports.iter().all(|report| !report.is_kept() && report.finding_count() == 1));
        assert!(reports.iter().all(|report| report.config.severity == Severity::Warn));
    }
}
