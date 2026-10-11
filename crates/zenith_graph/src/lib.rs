//! The zenith module tree and import graph.
//!
//! A [`Graph`] holds every internal directory and file of a project, every
//! external package and builtin that is imported, and the imports between
//! them, each with the [`ImportDetail`]s of the statements that make it. A
//! [`GraphBuilder`] assembles it as files are parsed and their imports
//! resolved.
//!
//! The graph is immutable once built. A contract that ignores some imports
//! checks against a [`GraphView`] that leaves them out, instead of mutating or
//! copying the graph. [`NodeSet`] and [`KindSet`] are the compact sets that
//! contracts and queries pass around.
//!
//! The queries that contracts run on a view are ports of
//! [grimp](https://github.com/python-grimp/grimp), the engine behind Python's
//! import-linter: shortest import chains ([`find_shortest_chains`]).

mod builder;
mod graph;
mod kinds;
mod node_set;
mod pathfinding;
#[cfg(test)]
mod testing;
mod view;

pub use builder::{FileHandle, GraphBuilder, GraphError, NodeHandle};
pub use graph::{Graph, ImportDetail, ModuleId, Node, NodeKind};
pub use kinds::{ImportKind, KindSet};
pub use node_set::NodeSet;
pub use pathfinding::{find_downstream, find_shortest_chain, find_shortest_chains};
pub use view::{EdgeSet, GraphView};
