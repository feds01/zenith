//! What contracts report: violations, import chains and per-contract results.

use std::time::Duration;

use serde::Serialize;
use zenith_config::Severity;
use zenith_graph::{GraphView, ImportKind, KindSet, ModuleId};

/// One import statement of a [Link].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkImport {
    pub line: u32,
    pub column: u32,
    pub specifier: String,
    pub kind: ImportKind,
    /// The byte span of the import in the importing file.
    #[serde(skip)]
    pub start: u32,
    #[serde(skip)]
    pub end: u32,
}

/// A direct import between two nodes, with every statement that makes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    /// The importing file, relative to the root.
    pub importer: String,
    /// The imported node: a file relative to the root, or `npm:`/`node:`.
    pub imported: String,
    /// The import statements, in source order.
    pub imports: Vec<LinkImport>,
}

impl Link {
    /// Build a link, keeping only the statements whose kind is not excluded.
    pub(crate) fn new(
        view: &GraphView,
        excluded_kinds: KindSet,
        importer: ModuleId,
        imported: ModuleId,
    ) -> Self {
        let graph = view.graph();
        let imports = graph
            .details(importer, imported)
            .iter()
            .filter(|detail| !excluded_kinds.contains(detail.kind))
            .map(|detail| LinkImport {
                line: detail.line,
                column: detail.column,
                specifier: detail.specifier.clone(),
                kind: detail.kind,
                start: detail.start,
                end: detail.end,
            })
            .collect();
        Self {
            importer: graph.name(importer).to_string(),
            imported: graph.name(imported).to_string(),
            imports,
        }
    }

    /// The baseline entry for this link: `importer -> imported`.
    pub fn key(&self) -> String {
        format!("{} -> {}", self.importer, self.imported)
    }
}

/// An import chain: a sequence of links where each link's `imported` is the
/// next link's `importer`.
///
/// For layer violations, a chain may stand for several equivalent routes
/// (import-linter's notation): `extra_heads` are other links into the first
/// middle module, and `extra_tails` are other links out of the last one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Chain {
    pub links: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_heads: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_tails: Vec<Link>,
}

impl Chain {
    /// Build a chain from a path of nodes.
    pub(crate) fn from_path(view: &GraphView, excluded_kinds: KindSet, path: &[ModuleId]) -> Self {
        let links =
            path.windows(2).map(|pair| Link::new(view, excluded_kinds, pair[0], pair[1])).collect();
        Self { links, extra_heads: Vec::new(), extra_tails: Vec::new() }
    }

    /// The links the downstream module's own code is responsible for: the
    /// first link, and any extra heads. These are what the baseline records.
    pub fn first_links(&self) -> impl Iterator<Item = &Link> {
        self.links.first().into_iter().chain(&self.extra_heads)
    }
}

/// A unit-to-unit dependency that should be removed to break a cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CycleBreaker {
    /// The depending unit.
    pub importer: String,
    /// The unit depended upon.
    pub imported: String,
    /// The direct imports that make up the dependency.
    pub imports: Vec<Link>,
    /// An example cycle through this dependency, as unit labels, starting
    /// and ending with `importer`.
    pub cycle: Vec<String>,
}

/// One violation of a contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Violation {
    /// `importer` is not allowed to import `imported`, but does through the
    /// given chains. Reported by `forbidden`, `independence` and `layers`.
    Dependency { importer: String, imported: String, chains: Vec<Chain> },
    /// Direct imports into `module` that are not allowed. Reported by
    /// `protected` and `encapsulation`.
    IllegalImports { module: String, imports: Vec<Link> },
    /// The units in `scope` have dependency cycles; removing the breakers
    /// would make them acyclic. Reported by `acyclic`.
    Cycle { scope: String, breakers: Vec<CycleBreaker> },
    /// A child of a container is not declared as a layer. Reported by
    /// exhaustive `layers` contracts.
    UndeclaredModule { container: String, module: String },
}

impl Violation {
    /// The direct imports that, when ignored, would resolve this violation
    /// (or at least make progress towards resolving it, for chains).
    pub fn baseline_links(&self) -> Vec<&Link> {
        match self {
            Violation::Dependency { chains, .. } => {
                chains.iter().flat_map(Chain::first_links).collect()
            }
            Violation::IllegalImports { imports, .. } => imports.iter().collect(),
            Violation::Cycle { breakers, .. } => breakers.iter().flat_map(|b| &b.imports).collect(),
            Violation::UndeclaredModule { .. } => Vec::new(),
        }
    }
}

/// Whether a contract was kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Kept,
    Broken,
}

/// The result of checking one contract.
#[derive(Debug, Clone, Serialize)]
pub struct ContractReport {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(serialize_with = "serialize_severity")]
    pub severity: Severity,
    pub status: Status,
    pub violations: Vec<Violation>,
    /// Problems with the contract's own configuration that are not fatal,
    /// e.g. `ignore_imports` entries that match nothing.
    pub warnings: Vec<String>,
    /// Like `warnings`, but they break the contract.
    pub errors: Vec<String>,
    /// Baseline entries whose import no longer exists.
    pub stale_baseline: Vec<String>,
    pub ignored_imports: usize,
    pub baselined_imports: usize,
    pub suppressed_imports: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
    #[serde(skip)]
    pub duration: Duration,
}

impl ContractReport {
    /// The number of individual findings: chains, illegal imports, cycle
    /// breakers and undeclared modules, plus configuration errors.
    pub fn finding_count(&self) -> usize {
        let violations: usize = self
            .violations
            .iter()
            .map(|violation| match violation {
                Violation::Dependency { chains, .. } => chains.len(),
                Violation::IllegalImports { imports, .. } => imports.len(),
                Violation::Cycle { breakers, .. } => breakers.len(),
                Violation::UndeclaredModule { .. } => 1,
            })
            .sum();
        violations + self.errors.len()
    }

    /// Whether this contract fails the check.
    pub fn fails(&self, stale_is_error: bool) -> bool {
        self.severity == Severity::Error
            && (self.status == Status::Broken
                || (stale_is_error && !self.stale_baseline.is_empty()))
    }
}

fn serialize_severity<S: serde::Serializer>(
    severity: &Severity,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(match severity {
        Severity::Error => "error",
        Severity::Warn => "warn",
    })
}
