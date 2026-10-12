//! Helpers for the crate's unit tests.

use thin_vec::thin_vec;
use zenith_config::Config;
use zenith_graph::{Graph, GraphBuilder, ImportDetail, ImportKind, NodeHandle};
use zenith_utils::location::ByteRange;
use zenith_workspace::Workspace;

use crate::{Chain, Contract, ContractError, ContractReport, Link, Violation};

/// A project for testing contracts: a graph, no workspace packages, and a
/// configuration.
pub(crate) struct Project {
    /// The module tree and import graph.
    pub graph: Graph,

    /// The workspace packages, of which there are none.
    pub workspace: Workspace,

    /// The configuration, with the contracts to test.
    pub config: Config,
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
            let detail = ImportDetail { range, specifier: format!("./import-{line}"), kind: *kind };
            builder.add_import(importer, imported, detail);
        }
        for client in clients {
            let client = builder.add_file(client).unwrap();
            builder.set_directives(client, thin_vec!["use client".to_string()]);
        }
        let config = Config::parse(toml).unwrap_or_else(|errors| panic!("{errors:?}"));
        Self { graph: builder.finish(), workspace: Workspace::default(), config }
    }

    /// Compile every contract.
    pub(crate) fn compile(&self) -> Result<Vec<Contract<'_>>, Vec<ContractError>> {
        Contract::compile_all(&self.config.contracts, &self.graph, &self.workspace)
    }

    /// Compile and check the contract `id`.
    pub(crate) fn check(&self, id: &str) -> ContractReport<'_> {
        let config = self.config.contract(id).unwrap();
        let contract = Contract::compile(config, &self.graph, &self.workspace).unwrap();
        contract.check(&self.graph, &self.workspace)
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
            Violation::Cycle { scope, breakers } => breakers
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
                .collect(),
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
