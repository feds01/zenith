//! The zenith module tree and import graph.
//!
//! The graph holds every internal directory and file of a project, every
//! external package and builtin that is imported, and the imports between
//! them. The query algorithms (shortest import chains, layer violations and
//! cycle breakers) are ports of [grimp](https://github.com/python-grimp/grimp),
//! the engine behind Python's import-linter.
//!
//! The graph is immutable once built. Contracts that need to ignore imports
//! do so through a [GraphView] rather than by mutating or cloning the graph.

mod cycles;
mod graph;
mod kinds;
mod layers;
mod node_set;
mod pathfinding;
mod view;

pub use cycles::{UnitGraph, find_unit_path, nominate_cycle_breakers};
pub use graph::{Graph, GraphBuilder, ImportDetail, ModuleId, Node, NodeKind};
pub use kinds::{ImportKind, KindSet};
pub use layers::{
    LayerRef, Level, PackageDependency, Route, find_illegal_dependencies_for_layers,
    find_illegal_direct_dependencies_for_layers,
};
pub use node_set::NodeSet;
pub use pathfinding::{find_downstream, find_shortest_chain, find_shortest_chains};
pub use view::{EdgeSet, GraphView};
