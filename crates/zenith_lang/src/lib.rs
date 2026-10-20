//! The interface that every language zenith supports implements, and the
//! types that languages share: what a file imports, its suppression comments
//! and syntax errors, and where each import resolves to.

mod resolved;
mod workspace;

pub use resolved::{ResolveFailure, Resolved, UnresolvedReason};
pub use workspace::{Workspace, WorkspacePackage};
