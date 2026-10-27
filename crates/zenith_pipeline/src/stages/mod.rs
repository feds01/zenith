//! The standard stages, one module each. Each is a thin adapter: the work
//! is done by the crate it wraps, so those crates don't depend on the
//! pipeline.

mod discover;
mod graph;
mod parse;
mod resolve;

pub use discover::{ConfigWalk, Discover};
pub use graph::BuildGraph;
pub use parse::Parse;
pub use resolve::Resolve;
