//! Terraform support: reading the module sources of Terraform files, and the
//! sources and dependencies of Terragrunt files, and resolving them to the
//! directories of the project or to modules from outside it.

pub mod parse;
pub mod resolve;
