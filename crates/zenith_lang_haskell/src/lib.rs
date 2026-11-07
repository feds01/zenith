//! Haskell support: the packages of a project from their `.cabal`
//! descriptions, the libraries that its builds installed, the module header
//! and the imports of each file read with zenith's own parser, their
//! resolution as GHC would resolve them, and the `[languages.haskell]` table
//! of the configuration.

mod backend;
mod config;
mod packages;
pub mod parse;
mod resolve;
#[cfg(test)]
mod testing;

pub use config::{HASKELL, HaskellConfig};
