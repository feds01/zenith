//! Go support: reading the imports of each file, and resolving import paths
//! against the Go modules of the project.

pub mod modules;
pub mod parse;
pub mod resolve;
