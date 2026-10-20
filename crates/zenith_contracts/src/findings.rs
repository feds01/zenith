//! What contracts find: violations, and the imports and chains of imports
//! that make them.
//!
//! Findings refer to nodes by their [`ModuleId`] in the graph they were found
//! in, and keep each import's [`ImportDetail`]s, so that a report can name the
//! files and show where in them each import is.

use std::fmt;

use zenith_graph::{Graph, GraphView, ImportDetail, ModuleId, Route};
use zenith_pipeline::Scope;

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
        let links = imports.map(|pair| Link::new(view, statements, pair[0], pair[1]));
        Self { links: links.collect(), extra_heads: Vec::new(), extra_tails: Vec::new() }
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
        Self {
            extra_heads: heads[1..].iter().map(|head| link(*head, *first)).collect(),
            extra_tails: imported.map(|tail| link(*last, *tail)).collect(),
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
}

impl Violation {
    /// The number of findings in the violation: its chains, its imports, its
    /// cycle breakers, or the one undeclared module.
    pub fn finding_count(&self) -> usize {
        match self {
            Self::Dependency { chains, .. } => chains.len(),
            Self::IllegalImports { imports, .. } => imports.len(),
            Self::Cycle { breakers, .. } => breakers.len(),
            Self::UndeclaredModule { .. } => 1,
        }
    }

    /// Keep only the findings that involve a node in `scope`, in `graph`,
    /// the graph they were found in, and return whether any is left:
    ///
    /// - the chains with an import from or into it, as a link or an extra head
    ///   or tail;
    /// - the illegal imports from or into it;
    /// - the cycle breakers with an import from or into it;
    /// - an undeclared module in it, or with a node of it inside.
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
        }
    }

    /// The direct imports that a baseline records to leave the violation
    /// out: the first links of its chains, made by the importing unit's own
    /// code, the illegal imports, or the imports of the cycle breakers. An
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
        }
    }
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
        match self {
            Self::Modules(selectors) => write!(f, "in `{selectors}`"),
            Self::Children(unit) => write!(f, "among the children of `{unit}`"),
        }
    }
}
