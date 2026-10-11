//! Specifier resolution.
//!
//! A [`Resolver`] resolves import specifiers with
//! [`oxc_resolver`](https://crates.io/crates/oxc_resolver), configured to see
//! what TypeScript and bundlers see: the nearest `tsconfig.json` of each file
//! (with `paths`, `extends` and project references), `exports` maps, `.js`
//! imports of `.ts` files, and symlinked workspace packages.
//!
//! Every resolution is then classified as an internal file, an external
//! package, a builtin, or unresolved (see [`Resolved`]).

mod resolver;
mod settings;
mod specifier;

pub use resolver::{ResolveFailure, Resolved, Resolver, ResolverError, UnresolvedReason};
pub use settings::{
    DEFAULT_CONDITIONS, DEFAULT_EXTENSIONS, DEFAULT_MAIN_FIELDS, ResolverSettings, TsconfigMode,
};
