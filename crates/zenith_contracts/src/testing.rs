//! Helpers for the crate's unit tests.

use thin_vec::{ThinVec, thin_vec};
use zenith_config::Config;
use zenith_graph::{Graph, GraphBuilder, ImportDetail, ImportKind, NodeHandle};
use zenith_parse::Suppression;
use zenith_pipeline::FileSuppressions;
use zenith_utils::location::{ByteRange, Located};
use zenith_workspace::Workspace;

use crate::{
    Baseline, Chain, Contract, ContractError, ContractReport, CycleScope, Exemptions, Link,
    UnusedSuppression, Violation,
};

/// A project for testing contracts: a graph, no workspace packages, a
/// configuration, suppressions and a baseline.
pub(crate) struct Project {
    /// The module tree and import graph.
    pub graph: Graph,

    /// The workspace packages, of which there are none.
    pub workspace: Workspace,

    /// The configuration, with the contracts to test.
    pub config: Config,

    /// The suppressions, sorted by file.
    pub suppressions: Vec<FileSuppressions>,

    /// The baseline.
    pub baseline: Baseline,
}

/// Value imports, for [`Project::new`].
pub(crate) fn values<'a>(pairs: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str, ImportKind)> {
    pairs.iter().map(|(importer, imported)| (*importer, *imported, ImportKind::Value)).collect()
}

impl Project {
    /// A project of the `(importer, imported, kind)` imports, each on a line
    /// of its own, configured by `toml`. An imported name starting with
    /// `npm:` is an external package, one starting with `node:` is a builtin,
    /// and anything else is a file. The `clients` files start with
    /// `"use client"`.
    pub(crate) fn new(imports: &[(&str, &str, ImportKind)], clients: &[&str], toml: &str) -> Self {
        let directives: Vec<_> = clients.iter().map(|client| (*client, "use client")).collect();
        Self::with_directives(imports, &directives, toml)
    }

    /// A project as [`Project::new`] makes it, where each `(file, directive)`
    /// file starts with that directive.
    pub(crate) fn with_directives(
        imports: &[(&str, &str, ImportKind)],
        directives: &[(&str, &str)],
        toml: &str,
    ) -> Self {
        let mut builder = GraphBuilder::new();
        for (line, (importer, imported, kind)) in (1..).zip(imports) {
            let importer = builder.add_file(importer).unwrap();
            let imported: NodeHandle = if imported.starts_with("npm:") {
                builder.add_external(imported).unwrap()
            } else if imported.starts_with("node:") {
                builder.add_builtin(imported).unwrap()
            } else {
                builder.add_file(imported).unwrap().into()
            };
            let range = ByteRange::new(line * 100, line * 100 + 10);
            let detail =
                ImportDetail { range, specifier: format!("./import-{line}").into(), kind: *kind };
            builder.add_import(importer, imported, detail);
        }
        for (file, directive) in directives {
            let file = builder.add_file(file).unwrap();
            builder.set_directives(file, thin_vec![(*directive).to_string()]);
        }
        let config = Config::parse(toml).unwrap_or_else(|errors| panic!("{errors:?}"));
        let workspace = Workspace::default();
        let (suppressions, baseline) = (Vec::new(), Baseline::default());
        Self { graph: builder.finish(), workspace, config, suppressions, baseline }
    }

    /// Add a suppression in `file`, of the space-separated `contracts`, to
    /// the imports on `line`, as [`Project::new`] numbers them.
    pub(crate) fn suppress(mut self, file: &str, line: u32, contracts: &str) -> Self {
        let contracts = contracts.split_whitespace().map(str::to_string).collect();
        let suppression = Suppression { contracts, reason: None, target: line_range(line) };
        let suppression = Located::new(ByteRange::new(0, 10), suppression);
        match self.suppressions.iter_mut().find(|suppressions| suppressions.file == file) {
            Some(suppressions) => suppressions.suppressions.push(suppression),
            None => {
                let suppressions: ThinVec<_> = thin_vec![suppression];
                self.suppressions.push(FileSuppressions { file: file.to_string(), suppressions });
                self.suppressions.sort_by(|a, b| a.file.cmp(&b.file));
            }
        }
        self
    }

    /// Set the baseline entries of the contract `id`.
    pub(crate) fn baseline(mut self, id: &str, entries: &[&str]) -> Self {
        let entries = entries.iter().map(|entry| entry.to_string()).collect();
        self.baseline.contracts.insert(id.to_string(), entries);
        self
    }

    /// What the project exempts from its contracts: its suppressions and
    /// baseline.
    pub(crate) fn exemptions(&self) -> Exemptions<'_> {
        Exemptions { suppressions: &self.suppressions, baseline: Some(&self.baseline) }
    }

    /// The baseline entries that keep the contract `id`.
    pub(crate) fn baseline_entries(&self, id: &str) -> Vec<String> {
        let config = self.config.contract(id).unwrap();
        let contract = Contract::compile(config, &self.graph, &self.workspace).unwrap();
        contract.baseline_entries(&self.graph, &self.workspace, &self.exemptions())
    }

    /// Compile every contract.
    pub(crate) fn compile(&self) -> Result<Vec<Contract<'_>>, Vec<ContractError>> {
        Contract::compile_all(&self.config.contracts, &self.graph, &self.workspace)
    }

    /// Compile and check the contract `id`.
    pub(crate) fn check(&self, id: &str) -> ContractReport<'_> {
        let config = self.config.contract(id).unwrap();
        let contract = Contract::compile(config, &self.graph, &self.workspace).unwrap();
        contract.check(&self.graph, &self.workspace, &self.exemptions())
    }

    /// The suppressions that had no effect on the contracts `ids`.
    pub(crate) fn unused_suppressions(&self, ids: &[&str]) -> Vec<UnusedSuppression<'_>> {
        let configs = ids.iter().map(|id| self.config.contract(id).unwrap());
        let contracts = Contract::compile_all(configs, &self.graph, &self.workspace).unwrap();
        let exemptions = self.exemptions();
        let reports = Contract::check_all(&contracts, &self.graph, &self.workspace, &exemptions);
        UnusedSuppression::find(&self.config, &exemptions, &reports)
    }

    /// The findings of the contract `id`, one line each: a chain as
    /// `<importer unit> -> <imported unit>: a.ts -> b.ts -> npm:c`, an
    /// illegal import as `<unit>: a.ts -> b.ts`, a cycle breaker as `<scope>:
    /// break <importer> -> <imported> (<imports>) in <cycle>`, and an
    /// undeclared module as ``<container>: `<module>` is not a layer``.
    pub(crate) fn findings(&self, id: &str) -> Vec<String> {
        let report = self.check(id);
        report.violations.iter().flat_map(|violation| self.describe(violation)).collect()
    }

    /// The findings of `violation`, as [`Project::findings`] shows them.
    pub(crate) fn describe(&self, violation: &Violation) -> Vec<String> {
        match violation {
            Violation::Dependency { importer, imported, chains } => chains
                .iter()
                .map(|chain| format!("{importer} -> {imported}: {}", self.path(chain)))
                .collect(),
            Violation::IllegalImports { module, imports } => {
                imports.iter().map(|link| format!("{module}: {}", self.link(link))).collect()
            }
            Violation::Cycle { scope, breakers } => {
                let scope = match scope {
                    CycleScope::Modules(selectors) => selectors.clone(),
                    CycleScope::Children(unit) => format!("children of {unit}"),
                };
                breakers
                    .iter()
                    .map(|breaker| {
                        let imports: Vec<_> =
                            breaker.imports.iter().map(|link| self.link(link)).collect();
                        format!(
                            "{scope}: break {} -> {} ({}) in {}",
                            breaker.importer,
                            breaker.imported,
                            imports.join(", "),
                            breaker.cycle.join(" -> ")
                        )
                    })
                    .collect()
            }
            Violation::UndeclaredModule { container, module } => {
                vec![format!("{container}: `{}` is not a layer", self.graph.name(*module))]
            }
        }
    }

    /// The nodes of `chain`, as `a.ts -> b.ts -> npm:c`.
    fn path(&self, chain: &Chain) -> String {
        let mut names: Vec<_> =
            chain.links.iter().map(|link| self.graph.name(link.importer)).collect();
        names.extend(chain.links.last().map(|link| self.graph.name(link.imported)));
        names.join(" -> ")
    }

    /// `link`, as `a.ts -> b.ts`.
    fn link(&self, link: &Link) -> String {
        format!("{} -> {}", self.graph.name(link.importer), self.graph.name(link.imported))
    }
}

/// The bytes of `line`, as [`Project::new`] numbers the lines of imports.
fn line_range(line: u32) -> ByteRange {
    ByteRange::new(line * 100, line * 100 + 100)
}
