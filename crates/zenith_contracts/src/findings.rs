//! What contracts find: violations, and the imports and chains of imports
//! that make them.
//!
//! Findings refer to nodes by their [`ModuleId`] in the graph they were found
//! in, and keep each import's [`ImportDetail`]s, so that a report can name the
//! files and show where in them each import is.

use zenith_graph::{GraphView, ImportDetail, KindSet, ModuleId};

/// A direct import between two nodes, with the statements that make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The importing file.
    pub importer: ModuleId,

    /// The imported node: a file, an external package or a builtin.
    pub imported: ModuleId,

    /// The statements that make the import, in source order, without those
    /// of the kinds that the contract excludes.
    pub imports: Vec<ImportDetail>,
}

impl Link {
    /// The import from `importer` to `imported` in `view`, with its
    /// statements that aren't of the `excluded` kinds.
    pub(crate) fn new(
        view: &GraphView,
        excluded: KindSet,
        importer: ModuleId,
        imported: ModuleId,
    ) -> Self {
        let details = view.graph().details(importer, imported).iter();
        let imports =
            details.filter(|detail| !excluded.contains(detail.kind.into())).cloned().collect();
        Self { importer, imported, imports }
    }
}

/// A chain of imports, each link's `imported` being the next link's
/// `importer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    /// The links, from the first importer to the last imported node. There
    /// is at least one.
    pub links: Vec<Link>,
}

impl Chain {
    /// The chain through the nodes of `path` in `view`, whose links keep only
    /// the statements that aren't of the `excluded` kinds.
    pub(crate) fn from_path(view: &GraphView, excluded: KindSet, path: &[ModuleId]) -> Self {
        let links = path.windows(2).map(|pair| Link::new(view, excluded, pair[0], pair[1]));
        Self { links: links.collect() }
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
}

impl Violation {
    /// The number of findings in the violation: its chains, or its imports.
    pub fn finding_count(&self) -> usize {
        match self {
            Self::Dependency { chains, .. } => chains.len(),
            Self::IllegalImports { imports, .. } => imports.len(),
        }
    }
}
