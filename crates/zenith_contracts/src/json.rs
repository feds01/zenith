//! What checking contracts finds, as the JSON output of `zenith check` shows
//! it: the checked contracts, and the suppressions that had no effect.

use serde::Serialize;
use zenith_config::{AlertLevel, Severity};
use zenith_diagnostics::{Position, SourceMap};
use zenith_graph::{Graph, ImportKind};
use zenith_reporting::milliseconds;

use crate::{
    Chain, ContractReport, CycleBreaker, CycleScope, Link, UnusedReason, UnusedSuppression,
    Violation, reports::import_span,
};

/// A checked contract.
#[derive(Debug, Serialize)]
pub struct ContractJson<'a> {
    /// The id.
    id: &'a str,

    /// The title.
    name: &'a str,

    /// The contract type.
    #[serde(rename = "type")]
    kind: &'static str,

    /// How severe it is when the contract is broken.
    severity: Severity,

    /// Whether the contract is kept.
    status: Status,

    /// How to fix its violations, if the configuration says.
    guidance: Option<&'a str>,

    /// How it is broken.
    violations: Vec<ViolationJson<'a>>,

    /// The `ignore_imports` entries that match no import, unless they are
    /// not reported.
    unmatched_ignore_imports: Vec<&'a str>,

    /// The baseline entries whose import no longer exists.
    stale_baseline_entries: &'a [String],

    /// The number of imports that `ignore_imports` left out.
    ignored: usize,

    /// The number of import statements that suppressions left out.
    suppressed: usize,

    /// The number of imports that the baseline left out.
    baselined: usize,

    /// How long checking it took, in milliseconds, if timings are shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    milliseconds: Option<f64>,
}

/// Whether a contract is kept.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    /// Nothing breaks it.
    Kept,

    /// It is broken.
    Broken,
}

/// How a contract is broken, tagged by `kind`.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ViolationJson<'a> {
    /// The unit `importer` depends on the unit `imported` through `chains`.
    Dependency { importer: &'a str, imported: &'a str, chains: Vec<ChainJson<'a>> },

    /// The unit `module` is imported directly by code that may not import it.
    IllegalImports { module: &'a str, imports: Vec<LinkJson<'a>> },

    /// The units of `scope` form cycles, which removing `breakers` would
    /// break.
    Cycle { scope: Scope<'a>, breakers: Vec<Breaker<'a>> },

    /// The child `module` of `container` is not declared as a layer.
    UndeclaredModule { container: &'a str, module: &'a str },
}

/// The units that an `acyclic` contract finds cycles among: `{"modules":
/// "pkg:*"}` or `{"children_of": "web/src"}`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Scope<'a> {
    /// The modules of the contract, as its selectors are written.
    Modules(&'a str),

    /// The children of the unit with this label.
    ChildrenOf(&'a str),
}

/// A chain of imports.
#[derive(Debug, Serialize)]
struct ChainJson<'a> {
    /// The links, from the first importer to the last imported node.
    links: Vec<LinkJson<'a>>,

    /// The other imports into the second node of the chain.
    extra_heads: Vec<LinkJson<'a>>,

    /// The other imports from the second-to-last node of the chain.
    extra_tails: Vec<LinkJson<'a>>,
}

/// A dependency to remove to break cycles.
#[derive(Debug, Serialize)]
struct Breaker<'a> {
    /// The label of the importing unit.
    importer: &'a str,

    /// The label of the imported unit.
    imported: &'a str,

    /// A cycle through the dependency, as the labels of its units, starting
    /// and ending with the importer.
    cycle: Vec<&'a str>,

    /// The direct imports that make the dependency.
    imports: Vec<LinkJson<'a>>,
}

/// A direct import between two nodes.
#[derive(Debug, Serialize)]
struct LinkJson<'a> {
    /// The importing file.
    importer: &'a str,

    /// The imported node.
    imported: &'a str,

    /// The statements that make the import, where their specifiers are.
    imports: Vec<Statement<'a>>,
}

/// An import statement.
#[derive(Debug, Serialize)]
struct Statement<'a> {
    /// The 1-based line of its specifier, if the file can be read.
    line: Option<u32>,

    /// The 1-based column of its specifier, if the file can be read.
    column: Option<u32>,

    /// The specifier, as written.
    specifier: &'a str,

    /// What kind of import it is.
    kind: ImportKind,
}

/// A suppression that had no effect.
#[derive(Debug, Serialize)]
pub struct UnusedSuppressionJson<'a> {
    /// The file it is in.
    file: &'a str,

    /// The 1-based line of the comment, if the file can be read.
    line: Option<u32>,

    /// The 1-based column of the comment, if the file can be read.
    column: Option<u32>,

    /// The contracts it names.
    contracts: &'a [String],

    /// Why it had no effect: `no_contract`, `unknown_contracts` or
    /// `no_import`.
    reason: &'static str,

    /// The reason, as `zenith check` shows it.
    message: String,
}

impl ContractReport<'_> {
    /// The contract and what checking it against `graph` found, as JSON,
    /// with where its imports are in `sources`, and how long checking it
    /// took if `timed`.
    pub fn json<'r>(
        &'r self,
        graph: &'r Graph,
        sources: &mut SourceMap,
        timed: bool,
    ) -> ContractJson<'r> {
        let config = self.config;
        let unmatched = match config.unmatched_ignore_imports {
            AlertLevel::Error | AlertLevel::Warn => &self.unmatched_ignores[..],
            AlertLevel::None => &[],
        };
        ContractJson {
            id: &config.id,
            name: &config.name,
            kind: config.kind.type_name(),
            severity: config.severity,
            status: if self.is_kept() { Status::Kept } else { Status::Broken },
            guidance: config.guidance.as_deref().map(str::trim),
            violations: self
                .violations
                .iter()
                .map(|violation| violation_json(graph, sources, violation))
                .collect(),
            unmatched_ignore_imports: unmatched.iter().map(|entry| entry.as_str()).collect(),
            stale_baseline_entries: &self.stale_entries,
            ignored: self.ignored,
            suppressed: self.suppressed,
            baselined: self.baselined,
            milliseconds: timed.then(|| milliseconds(self.duration)),
        }
    }
}

impl UnusedSuppression<'_> {
    /// The suppression as JSON, with where it is in `sources`.
    pub fn json(&self, sources: &mut SourceMap) -> UnusedSuppressionJson<'_> {
        let source = sources.file(self.file).map(|id| sources.get(id));
        let position = source.map(|source| source.position(self.suppression.range.start()));
        let (line, column) = split(position);
        UnusedSuppressionJson {
            file: self.file,
            line,
            column,
            contracts: &self.suppression.value.contracts,
            reason: match self.reason {
                UnusedReason::NoContract => "no_contract",
                UnusedReason::UnknownContracts(_) => "unknown_contracts",
                UnusedReason::NoImport => "no_import",
            },
            message: self.message(source),
        }
    }
}

/// `violation`, found in `graph`, as JSON.
fn violation_json<'r>(
    graph: &'r Graph,
    sources: &mut SourceMap,
    violation: &'r Violation,
) -> ViolationJson<'r> {
    let mut links = |links: &'r [Link]| -> Vec<LinkJson<'r>> {
        links.iter().map(|link| link_json(graph, sources, link)).collect()
    };
    match violation {
        Violation::Dependency { importer, imported, chains } => ViolationJson::Dependency {
            importer,
            imported,
            chains: chains
                .iter()
                .map(|chain: &'r Chain| ChainJson {
                    links: links(&chain.links),
                    extra_heads: links(&chain.extra_heads),
                    extra_tails: links(&chain.extra_tails),
                })
                .collect(),
        },
        Violation::IllegalImports { module, imports } => {
            ViolationJson::IllegalImports { module, imports: links(imports) }
        }
        Violation::Cycle { scope, breakers } => ViolationJson::Cycle {
            scope: match scope {
                CycleScope::Modules(selectors) => Scope::Modules(selectors),
                CycleScope::Children(unit) => Scope::ChildrenOf(unit),
            },
            breakers: breakers
                .iter()
                .map(|breaker: &'r CycleBreaker| Breaker {
                    importer: &breaker.importer,
                    imported: &breaker.imported,
                    cycle: breaker
                        .cycle
                        .iter()
                        .chain([&breaker.importer])
                        .map(String::as_str)
                        .collect(),
                    imports: links(&breaker.imports),
                })
                .collect(),
        },
        Violation::UndeclaredModule { container, module } => {
            ViolationJson::UndeclaredModule { container, module: graph.name(*module) }
        }
    }
}

/// `link`, of `graph`, as JSON, with where the specifiers of its statements
/// are in `sources`.
fn link_json<'r>(graph: &'r Graph, sources: &mut SourceMap, link: &'r Link) -> LinkJson<'r> {
    let importer = graph.name(link.importer);
    let source = sources.file(importer).map(|id| sources.get(id));
    let imports = link
        .imports
        .iter()
        .map(|import| {
            let position =
                source.map(|source| source.position(import_span(&source.contents, import).start()));
            let (line, column) = split(position);
            Statement { line, column, specifier: &import.specifier, kind: import.kind }
        })
        .collect();
    LinkJson { importer, imported: graph.name(link.imported), imports }
}

/// The line and column of `position`, if there is one.
fn split(position: Option<Position>) -> (Option<u32>, Option<u32>) {
    position.map_or((None, None), |position| (Some(position.line), Some(position.column)))
}
