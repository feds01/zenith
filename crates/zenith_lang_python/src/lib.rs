//! Python support: the packages of a project from its `pyproject.toml`
//! files, the imports of each file read with Ruff's parser, their resolution
//! against the roots that absolute imports start from, and the
//! `[languages.python]` table of the configuration.

mod backend;
mod config;
mod modules;
mod packages;
pub mod parse;
mod resolve;
mod stdlib;
#[cfg(test)]
mod testing;

pub use config::{PYTHON, PythonConfig};
