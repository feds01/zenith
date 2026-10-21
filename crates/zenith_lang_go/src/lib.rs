//! Go support: reading the imports of each file, resolving import paths
//! against the Go modules of the project, and the `[languages.go]` table of the
//! configuration.

mod backend;
mod config;
pub mod modules;
pub mod parse;
pub mod resolve;

pub use config::{GO, GoConfig};
