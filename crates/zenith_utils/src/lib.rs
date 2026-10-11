//! Utilities shared by the zenith crates: file reading and path helpers,
//! source locations and line indexes, what can be told about an import
//! specifier from its text, and the glob dialect that selectors are written
//! in.

pub mod fs;
pub mod glob;
pub mod location;
pub mod path;
pub mod specifier;
