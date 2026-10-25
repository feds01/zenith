//! Rust support: the crates of a Cargo workspace from its `Cargo.toml` files,
//! the imports of each file read with rust-analyzer's parser, in the edition
//! of its crate, their resolution along the
//! module tree of each crate, and the `[languages.rust]` table of the
//! configuration.

mod backend;
mod config;
mod crates;
mod modules;
mod parse;
mod resolve;
#[cfg(test)]
mod testing;

pub use config::{RUST, RustConfig};
