//! Architecture contracts, and the units they reason about.
//!
//! A contract is about the modules that its selectors pick. Each selector
//! resolves, against the import graph and workspace packages of the project,
//! into a [`Selection`] of [`Unit`](zenith_graph::Unit)s: labelled sets of
//! the nodes that take part in imports.

mod selection;

pub use selection::Selection;
