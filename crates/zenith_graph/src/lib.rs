//! The zenith module tree and import graph.
//!
//! A [`Graph`] holds every internal directory and file of a project, every
//! external package and builtin that is imported, and the imports between
//! them, each with the [`ImportDetail`]s of the statements that make it. A
//! [`GraphBuilder`] assembles it as files are parsed and their imports
//! resolved.
//!
//! An import usually targets a file, an external package or a builtin, but it
//! can also target a package: a directory that is imported as a whole, as in
//! Go, where an import names a directory. The edge is to the directory, from
//! the file that makes the import, and the directory never imports anything.
//! What a package reaches is what its direct files reach, which
//! [`GraphView`]'s searches follow without the graph storing an import for
//! it.
//!
//! The graph is immutable once built. A contract that ignores some imports
//! checks against a [`GraphView`] that leaves them out, instead of mutating or
//! copying the graph. [`NodeSet`] and [`KindSet`] are the compact sets that
//! contracts and queries pass around.
//!
//! The queries that contracts run on a view are ports of
//! [grimp](https://github.com/python-grimp/grimp), the engine behind Python's
//! import-linter: shortest import chains
//! ([`GraphView::find_shortest_chains`]), layered architectures
//! ([`GraphView::find_illegal_dependencies_for_layers`]) and cycle breakers
//! between units ([`UnitGraph::nominate_cycle_breakers`]).
//!
//! A [`ModuleOverview`] shows the shape of part of a project: a node, its
//! children, and the dependencies between them, as a tree, a Graphviz digraph,
//! a Mermaid flowchart or JSON. A [`DependencyDiff`] shows how those
//! dependencies changed from one graph to another.
//!
//! A [`Scope`] is the part of a project that a command reports on: the
//! nodes under some paths, or every node.

mod builder;
mod cycles;
mod diff;
mod exports;
mod find;
mod graph;
mod kinds;
mod layers;
mod node_set;
mod overview;
mod pathfinding;
mod reexports;
mod scope;
#[cfg(test)]
mod testing;
mod unit;
mod view;

pub use builder::{FileHandle, GraphBuilder, GraphError, NodeHandle, NodeIds};
pub use cycles::{UnitDependency, UnitGraph, UnitId};
pub use diff::DependencyDiff;
pub use exports::{Exports, ImportedName, ImportedNames, ReExport};
pub use graph::{Graph, ImportDetail, ModuleId, Node, NodeKind};
pub use kinds::{ImportKind, KindSet};
pub use layers::{Architecture, LayerId, LayerRef, Level, LevelId, PackageDependency, Route};
pub use node_set::NodeSet;
pub use overview::{
    ChildDependency, ModuleOverview, OverviewChild, OverviewDot, OverviewMermaid, OverviewTree,
};
pub use reexports::ModuleExports;
pub use scope::Scope;
pub use unit::Unit;
pub use view::{EdgeSet, GraphView};
