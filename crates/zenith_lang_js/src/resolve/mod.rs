//! Specifier resolution.
//!
//! A [`Resolver`] resolves import specifiers with
//! [`oxc_resolver`](https://crates.io/crates/oxc_resolver), configured to see
//! what TypeScript and bundlers see: the nearest `tsconfig.json` of each file
//! (with `paths`, `extends` and project references), `exports` maps, `.js`
//! imports of `.ts` files, and symlinked workspace packages. Wherever nothing
//! is installed, the workspace packages are linked into the root
//! `node_modules`, as a package manager would link them, so imports of them,
//! and `tsconfig.json` files that extend one from them, resolve the same
//! whether or not `node_modules` is installed, and never through the
//! `node_modules` of a directory that holds the project. An import of a
//! workspace package that still fails is resolved directly against the
//! package directory, through its `exports` map. A file whose `tsconfig.json`
//! can't be loaded, because it extends one from a package that isn't
//! installed say, is resolved without it.
//!
//! Every resolution is then classified as an internal file, an external
//! package, a builtin, or unresolved (see [`Resolved`](zenith_lang::Resolved)).

mod exports;
mod links;
mod resolver;
mod settings;

pub use resolver::{Resolver, ResolverError};
pub use settings::{
    DEFAULT_CONDITIONS, DEFAULT_EXTENSIONS, DEFAULT_MAIN_FIELDS, ResolverSettings, TsconfigMode,
};
