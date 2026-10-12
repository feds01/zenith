//! Utilities shared by the zenith crates: file reading and path helpers,
//! source locations and line indexes, what can be told about an import
//! specifier from its text, the glob dialect that selectors are written in,
//! terminal output: highlighting, logging, and printing text and trees, and
//! the global thread pool.

pub mod fs;
pub mod glob;
pub mod highlight;
pub mod location;
pub mod logging;
pub mod path;
pub mod pool;
pub mod printing;
pub mod specifier;
pub mod tree_writing;
