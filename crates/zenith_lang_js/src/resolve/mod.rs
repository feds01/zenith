//! Specifier resolution.
//!
//! A [`Resolver`] resolves import specifiers with
//! [`oxc_resolver`](https://crates.io/crates/oxc_resolver), configured to see
//! what TypeScript and bundlers see: the nearest `tsconfig.json` of each file
//! (with `paths`, `extends` and project references), `exports` maps, `.js`
//! imports of `.ts` files, and symlinked workspace packages. When
//! `node_modules` is not installed, imports of workspace packages are resolved
//! directly against the package directories, through their `exports` maps, so
//! they resolve the same either way. A file whose `tsconfig.json` can't be
//! loaded, because it extends one from a package that isn't installed say, is
//! resolved without it.
//!
//! Every resolution is then classified as an internal file, an external
//! package, a builtin, or unresolved (see [`Resolved`](zenith_lang::Resolved)).

mod exports;
mod resolver;
mod settings;

pub use resolver::{Resolver, ResolverError};
pub use settings::{
    DEFAULT_CONDITIONS, DEFAULT_EXTENSIONS, DEFAULT_MAIN_FIELDS, ResolverSettings, TsconfigMode,
};
