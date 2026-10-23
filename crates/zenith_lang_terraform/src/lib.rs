//! Terraform support: reading the module sources of Terraform files, and the
//! sources and dependencies of Terragrunt files, resolving them to the
//! directories of the project or to modules from outside it, and the
//! `[languages.terraform]` table of the configuration.

mod backend;
mod config;
pub mod parse;
pub mod resolve;

pub use config::{TERRAFORM, TerraformConfig};
