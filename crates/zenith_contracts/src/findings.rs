//! What contracts find: violations, the imports and chains of imports that
//! make them, and how to fix them.
//!
//! Findings refer to nodes by their [`ModuleId`] in the graph they were found
//! in, and keep each import's [`ImportDetail`]s, so that a report can name the
//! files and show where in them each import is.

use std::fmt;

use serde::Serialize;
use zenith_config::ContractKind;
use zenith_diagnostics::message;
use zenith_graph::{Graph, GraphView, ImportDetail, ModuleId, Route, Scope};

use crate::view::Statements;

/// A direct import between two nodes, with the statements that make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The importing file.
    pub importer: ModuleId,

    /// The imported node: a file, an external package, a builtin, or a
    /// package, a directory that is imported as a whole. A package is shown
    /// by its path, and an import of it is one link, however many files it
    /// has.
    pub imported: ModuleId,

    /// The statements that make the import, in source order, without those
    /// of the kinds that the contract excludes, or suppressed for it.
    pub imports: Vec<ImportDetail>,
}

impl Link {
    /// Whether the importer or the imported node, in `graph`, is in `scope`.
    /// An imported package is, if a file directly in it is.
    pub(crate) fn touches(&self, graph: &Graph, scope: &Scope) -> bool {
        scope.contains(graph.name(self.importer))
            || scope.contains(graph.name(self.imported))
            || graph.package_files(self.imported).any(|file| scope.contains(graph.name(file)))
    }

    /// The import from `importer` to `imported` in `view`, with its
    /// statements that count.
    pub(crate) fn new(
        view: &GraphView,
        statements: &Statements,
        importer: ModuleId,
        imported: ModuleId,
    ) -> Self {
        let details = view.graph().details(importer, imported).iter();
        let imports =
            details.filter(|detail| statements.count(importer, detail)).cloned().collect();
        Self { importer, imported, imports }
    }
}

/// A chain of imports, each link's `imported` being the next link's
/// `importer`, or, if it is a package, the package that the next link's
/// importer is a file directly in.
///
/// For example, if `cmd/main.go` imports the package `internal/db`, and
/// `internal/db/conn.go` imports the package `internal/cache`, the chain from
/// `cmd/main.go` to `internal/cache` has two links: `cmd/main.go ->
/// internal/db` and `internal/db/conn.go -> internal/cache`. What leads from
/// the package to the file in it is not an import, so no link stands for it.
///
/// A chain between two layers can stand for several routes that share their
/// middle, as in import-linter: `extra_heads` are the other imports into the
/// first node of the middle, from members of the importing layer, and
/// `extra_tails` the other imports from its last node, into members of the
/// imported layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    /// The links, from the first importer to the last imported node. There
    /// is at least one. Each is an import that the code makes.
    pub links: Vec<Link>,

    /// The other imports into the second node of the chain, in the order of
    /// their importers. Empty for a direct import.
    pub extra_heads: Vec<Link>,

    /// The other imports from the second-to-last node of the chain, in the
    /// order of the imported nodes. Empty for a direct import.
    pub extra_tails: Vec<Link>,
}

impl Chain {
    /// The nodes that the chain goes through, in order: the importer of each
    /// link, the package between two links if there is one, and the node that
    /// the last link imports.
    pub fn nodes(&self) -> Vec<ModuleId> {
        let mut nodes = Vec::with_capacity(self.links.len() + 1);
        for (index, link) in self.links.iter().enumerate() {
            nodes.push(link.importer);
            let next = self.links.get(index + 1);
            if next.is_some_and(|next| next.importer != link.imported) {
                nodes.push(link.imported);
            }
        }
        nodes.extend(self.links.last().map(|link| link.imported));
        nodes
    }

    /// The chain through the nodes of `path` in `view`, whose links keep only
    /// the statements that count. A hop from a package into one of its files
    /// is not an import, so it is not a link.
    pub(crate) fn from_path(view: &GraphView, statements: &Statements, path: &[ModuleId]) -> Self {
        let graph = view.graph();
        let packages = graph.has_packages();
        let imports =
            path.windows(2).filter(|pair| !packages || !graph.is_package_member(pair[0], pair[1]));
        // The filter hides how many links there are, which `collect` would then
        // guess, and over-allocate for the short chains that most are.
        let mut links = Vec::with_capacity(path.len().saturating_sub(1));
        links.extend(imports.map(|pair| Link::new(view, statements, pair[0], pair[1])));
        Self { links, extra_heads: Vec::new(), extra_tails: Vec::new() }
    }

    /// The chain of a route between two layers: through its first head, its
    /// middle and its first tail, with the other heads and tails as extra
    /// links.
    pub(crate) fn from_route(view: &GraphView, statements: &Statements, route: &Route) -> Self {
        let (heads, middle, tails) = (route.heads(), route.middle(), route.tails());
        let (Some(first), Some(last)) = (middle.first(), middle.last()) else {
            // A direct import has one head and one tail.
            return Self::from_path(view, statements, &[heads[0], tails[0]]);
        };
        let path: Vec<_> =
            [heads[0]].into_iter().chain(middle.iter().copied()).chain([tails[0]]).collect();
        let link = |importer, imported| Link::new(view, statements, importer, imported);
        // A package that ends the middle has the files in it as tails, which it does
        // not import.
        let graph = view.graph();
        let packages = graph.has_packages();
        let imported =
            tails[1..].iter().filter(|tail| !packages || !graph.is_package_member(*last, **tail));
        let mut extra_tails = Vec::with_capacity(tails.len() - 1);
        extra_tails.extend(imported.map(|tail| link(*last, *tail)));
        Self {
            extra_heads: heads[1..].iter().map(|head| link(*head, *first)).collect(),
            extra_tails,
            ..Self::from_path(view, statements, &path)
        }
    }
}

/// How a contract is broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// The unit `importer` may not depend on the unit `imported`, but does,
    /// through `chains`. A direct import is a chain of one link.
    Dependency {
        /// The label of the importing unit.
        importer: String,

        /// The label of the imported unit.
        imported: String,

        /// The chains, shortest first, then in the order of the nodes they go
        /// through. There is at least one.
        chains: Vec<Chain>,
    },

    /// The unit `module` is imported directly by code that may not import it.
    IllegalImports {
        /// The label of the imported unit.
        module: String,

        /// The imports, in the order of their importer, then of the imported
        /// member. There is at least one.
        imports: Vec<Link>,
    },

    /// The units of `scope` depend on each other in cycles, which removing
    /// the `breakers` would break.
    Cycle {
        /// The units that form cycles.
        scope: CycleScope,

        /// The dependencies to remove, in the order of their importing unit,
        /// then of the imported unit. There is at least one.
        breakers: Vec<CycleBreaker>,
    },

    /// A child of a container of an exhaustive `layers` contract is not
    /// declared as a layer.
    UndeclaredModule {
        /// The label of the container.
        container: String,

        /// The child that no layer declares.
        module: ModuleId,
    },

    /// The unit `module` has more importers, or imports more nodes, than a
    /// `limits` contract allows.
    LimitExceeded {
        /// The label of the unit.
        module: String,

        /// The limit that it is over.
        limit: Limit,

        /// The number of distinct nodes outside the unit that the limit counts.
        count: usize,

        /// The most that the limit allows, which `count` is over.
        max: usize,

        /// The imports that make up the count, in the order of the node that
        /// they count, then of their other node. There is at least one.
        imports: Vec<Link>,
    },
}

impl Violation {
    /// The number of findings in the violation: its chains, its imports, its
    /// cycle breakers, or the one undeclared module or unit over its limit.
    pub fn finding_count(&self) -> usize {
        match self {
            Self::Dependency { chains, .. } => chains.len(),
            Self::IllegalImports { imports, .. } => imports.len(),
            Self::Cycle { breakers, .. } => breakers.len(),
            Self::UndeclaredModule { .. } | Self::LimitExceeded { .. } => 1,
        }
    }

    /// Keep only the findings that involve a node in `scope`, in `graph`,
    /// the graph they were found in, and return whether any is left:
    ///
    /// - the chains with an import from or into it, as a link or an extra head
    ///   or tail;
    /// - the illegal imports from or into it;
    /// - the cycle breakers with an import from or into it;
    /// - an undeclared module in it, or with a node of it inside;
    /// - a unit over its limit, with every import that makes up the count, if
    ///   one of them is from or into it.
    pub(crate) fn narrow(&mut self, graph: &Graph, scope: &Scope) -> bool {
        let touches = |link: &Link| link.touches(graph, scope);
        match self {
            Self::Dependency { chains, .. } => {
                chains.retain(|chain| {
                    chain
                        .links
                        .iter()
                        .chain(&chain.extra_heads)
                        .chain(&chain.extra_tails)
                        .any(touches)
                });
                !chains.is_empty()
            }
            Self::IllegalImports { imports, .. } => {
                imports.retain(touches);
                !imports.is_empty()
            }
            Self::Cycle { breakers, .. } => {
                breakers.retain(|breaker| breaker.imports.iter().any(touches));
                !breakers.is_empty()
            }
            Self::UndeclaredModule { module, .. } => scope.overlaps(graph.name(*module)),
            Self::LimitExceeded { imports, .. } => imports.iter().any(touches),
        }
    }

    /// The direct imports that a baseline records to leave the violation
    /// out: the first links of its chains, made by the importing unit's own
    /// code, the illegal imports, the imports of the cycle breakers, or the
    /// imports of the counted nodes over the limit, the last ones. An
    /// undeclared module has none, as it is not about imports.
    pub(crate) fn baseline_links(&self) -> Vec<&Link> {
        match self {
            Self::Dependency { chains, .. } => chains
                .iter()
                .flat_map(|chain| chain.links.first().into_iter().chain(&chain.extra_heads))
                .collect(),
            Self::IllegalImports { imports, .. } => imports.iter().collect(),
            Self::Cycle { breakers, .. } => {
                breakers.iter().flat_map(|breaker| &breaker.imports).collect()
            }
            Self::UndeclaredModule { .. } => Vec::new(),
            Self::LimitExceeded { limit, max, imports, .. } => {
                limit.groups(imports).skip(*max).flatten().collect()
            }
        }
    }

    /// How to fix each finding of the violation, which a contract of type
    /// `kind` found, in order: a chain at its first import, which the
    /// importing unit's own code makes; an illegal import at it; a cycle
    /// breaker at its first import; and an undeclared module and a unit over
    /// a limit at no import.
    pub fn fixes(&self, kind: &ContractKind) -> Vec<Fix<'_>> {
        let action = match self {
            Self::Dependency { .. } | Self::IllegalImports { .. } => match kind {
                ContractKind::Forbidden(_) | ContractKind::Protected(_) => FixAction::RemoveImport,
                ContractKind::Independence(_) | ContractKind::Layers(_) => FixAction::MoveModule,
                ContractKind::Encapsulation(_) => FixAction::ImportThroughEntryPoint,
                // An `acyclic` contract finds cycles only, and a `limits` contract
                // units over a limit.
                ContractKind::Acyclic(_) => FixAction::BreakCycle,
                ContractKind::Limits(_) => FixAction::SplitModule,
            },
            Self::Cycle { .. } => FixAction::BreakCycle,
            Self::UndeclaredModule { .. } => FixAction::DeclareLayer,
            Self::LimitExceeded { .. } => FixAction::SplitModule,
        };
        let fix = |link, hint| Fix { action, link, hint };
        match self {
            Self::Dependency { chains, .. } => chains
                .iter()
                .map(|chain| {
                    let hint = match (action, chain.links.len()) {
                        (FixAction::MoveModule, _) => message!(contracts_fix_move_module),
                        (_, 1) => message!(contracts_fix_remove_import),
                        _ => message!(contracts_fix_remove_import.chain),
                    };
                    fix(chain.links.first(), hint)
                })
                .collect(),
            Self::IllegalImports { module, imports } => imports
                .iter()
                .map(|link| {
                    let hint = match action {
                        FixAction::ImportThroughEntryPoint => {
                            message!(contracts_fix_import_through_entry_point, module)
                        }
                        _ => message!(contracts_fix_remove_import),
                    };
                    fix(Some(link), hint)
                })
                .collect(),
            Self::Cycle { breakers, .. } => breakers
                .iter()
                .map(|breaker| {
                    let (importer, imported) = (&breaker.importer, &breaker.imported);
                    let hint = message!(contracts_fix_break_cycle, importer, imported);
                    fix(breaker.imports.first(), hint)
                })
                .collect(),
            Self::UndeclaredModule { .. } => vec![fix(None, message!(contracts_fix_declare_layer))],
            Self::LimitExceeded { limit, .. } => vec![fix(None, limit.hint())],
        }
    }

    /// How to fix all the findings of the violation, which a contract of
    /// type `kind` found, in a line: the hint of its one finding's fix, or
    /// else one that holds for each of its findings.
    pub fn hint(&self, kind: &ContractKind) -> String {
        let mut fixes = self.fixes(kind);
        match self {
            Self::UndeclaredModule { .. } => message!(contracts_undeclared_module.help),
            Self::LimitExceeded { limit, .. } => limit.hint_all(),
            _ if fixes.len() == 1 => fixes.remove(0).hint,
            Self::Dependency { chains, .. } => match fixes[0].action {
                FixAction::MoveModule => message!(contracts_fix_move_module.all),
                _ if chains.iter().any(|chain| chain.links.len() > 1) => {
                    message!(contracts_fix_remove_import.chains)
                }
                _ => message!(contracts_fix_remove_import.all),
            },
            Self::IllegalImports { .. } => match fixes[0].action {
                FixAction::ImportThroughEntryPoint => fixes.remove(0).hint,
                _ => message!(contracts_fix_remove_import.all),
            },
            Self::Cycle { .. } => message!(contracts_fix_break_cycle.all),
        }
    }
}

/// A limit of a `limits` contract: what it counts of a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// The files outside the unit that import it.
    FanIn,

    /// The files and packages of the project outside the unit that it
    /// imports.
    FanOut,
}

impl Limit {
    /// The node of `link`, an import into or from a unit, that the limit
    /// counts: its importer for fan-in, and the node it imports for fan-out.
    pub(crate) fn counted(self, link: &Link) -> ModuleId {
        match self {
            Self::FanIn => link.importer,
            Self::FanOut => link.imported,
        }
    }

    /// The name of the limit, as JSON and SARIF show it: `fan_in` or
    /// `fan_out`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::FanIn => "fan_in",
            Self::FanOut => "fan_out",
        }
    }

    /// `imports`, sorted by the node that the limit counts, in groups by that
    /// node. Each group counts once, so the count is the number of groups.
    pub(crate) fn groups(self, imports: &[Link]) -> impl Iterator<Item = &[Link]> {
        imports.chunk_by(move |a, b| self.counted(a) == self.counted(b))
    }

    /// What it says of the unit `module` that `count` is over the `max` of
    /// the limit.
    pub(crate) fn title(self, module: &str, count: usize, max: usize) -> String {
        match self {
            Self::FanIn => message!(contracts_fan_in, module, count, max),
            Self::FanOut => message!(contracts_fan_out, module, count, max),
        }
    }

    /// How to fix a unit over the limit, in a line. It never suggests
    /// raising the limit.
    fn hint(self) -> String {
        match self {
            Self::FanIn => message!(contracts_fix_split_module),
            Self::FanOut => message!(contracts_fix_split_module.fan_out),
        }
    }

    /// How to fix a unit over the limit, under the imports that a report of
    /// it lists.
    fn hint_all(self) -> String {
        match self {
            Self::FanIn => message!(contracts_fix_split_module.all),
            Self::FanOut => message!(contracts_fix_split_module.all_fan_out),
        }
    }
}

/// How to fix a finding of a violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix<'a> {
    /// What to do.
    pub action: FixAction,

    /// The import to change, if the fix is about one.
    pub link: Option<&'a Link>,

    /// What to do, in a line of plain language.
    pub hint: String,
}

/// What fixes a finding of a violation, named in the JSON output in
/// snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixAction {
    /// Remove the import, which a `forbidden` or `protected` contract
    /// forbids.
    RemoveImport,

    /// Move what the importer needs into a module that it may import, as
    /// `independence` and `layers` contracts allow, or remove the import.
    MoveModule,

    /// Import the module of an `encapsulation` contract through one of its
    /// entry points.
    ImportThroughEntryPoint,

    /// Remove the dependency, one that would make an `acyclic` contract's
    /// units acyclic.
    BreakCycle,

    /// Declare the module as a layer of an exhaustive `layers` contract, or
    /// ignore it.
    DeclareLayer,

    /// Split the unit of a `limits` contract, or remove some of the imports
    /// that make it go over its limit.
    SplitModule,
}

/// A dependency of one unit on another whose removal, with the other
/// breakers of its cycle, would make the units acyclic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleBreaker {
    /// The label of the importing unit.
    pub importer: String,

    /// The label of the imported unit.
    pub imported: String,

    /// The direct imports that make the dependency, in the order of their
    /// importer, then of the imported node. There is at least one.
    pub imports: Vec<Link>,

    /// A cycle through the dependency, as the labels of its units: the
    /// importer, the imported unit, and the units back to the importer, which
    /// is not repeated.
    pub cycle: Vec<String>,
}

/// The units that an `acyclic` contract finds cycles among.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleScope {
    /// The modules of the contract, shown as its selectors are written, e.g.
    /// `pkg:*`.
    Modules(String),

    /// The children of the unit with this label, checked because of the
    /// contract's `depth`.
    Children(String),
}

/// Shown as where the cycles are, e.g. ``in `pkg:*` `` or ``among the
/// children of `web/src` ``.
impl fmt::Display for CycleScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&match self {
            Self::Modules(selectors) => message!(contracts_cycle_in, selectors),
            Self::Children(unit) => message!(contracts_cycle_among_children, unit),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        FixAction,
        testing::{Project, values},
    };

    #[test]
    fn each_contract_type_fixes_its_violations_with_its_own_action() {
        let project = Project::new(
            &values(&[
                ("a/x.ts", "b/y.ts"),
                ("b/y.ts", "a/w.ts"),
                ("c/v.ts", "a/x.ts"),
                ("p/a/x.ts", "p/d/y.ts"),
            ]),
            &[],
            include_str!("fixtures/fixes.toml"),
        );
        let fix = |id: &str| {
            let report = project.check(id);
            let fix = report.violations[0].fixes(&report.config.kind).remove(0);
            let link = fix.link.map(|link| project.graph.name(link.importer).to_string());
            (fix.action, link)
        };
        let a = Some("a/x.ts".to_string());
        assert_eq!(fix("forbidden"), (FixAction::RemoveImport, a.clone()));
        assert_eq!(fix("independence"), (FixAction::MoveModule, a.clone()));
        assert_eq!(fix("layers"), (FixAction::MoveModule, a.clone()));
        assert_eq!(fix("exhaustive"), (FixAction::DeclareLayer, None));
        assert_eq!(fix("protected"), (FixAction::RemoveImport, a.clone()));
        assert_eq!(fix("acyclic"), (FixAction::BreakCycle, Some("b/y.ts".into())));
        assert_eq!(fix("encapsulation"), (FixAction::ImportThroughEntryPoint, a));
        assert_eq!(fix("limits"), (FixAction::SplitModule, None));
    }
}
