//! Utilities shared by the zenith crates: file reading and path helpers,
//! source locations and line indexes, what can be told about an import
//! specifier from its text, commenting out merge conflict markers, the glob
//! dialect that selectors are written in, terminal output: highlighting,
//! logging, and printing trees, writing TOML by hand, and the global thread
//! pool.

pub mod fs;
pub mod glob;
pub mod highlight;
pub mod location;
pub mod logging;
pub mod path;
pub mod pool;
pub mod specifier;
pub mod text;
pub mod toml_writing;
pub mod tree_writing;
