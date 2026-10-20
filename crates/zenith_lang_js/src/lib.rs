//! JavaScript and TypeScript support: reading what each file imports and
//! exports with oxc, and resolving its specifiers.

mod packages;
mod parse;
mod resolve;

pub use packages::discover_packages;
pub use parse::{ParseOptions, Parser};
pub use resolve::{
    DEFAULT_CONDITIONS, DEFAULT_EXTENSIONS, DEFAULT_MAIN_FIELDS, Resolver, ResolverError,
    ResolverSettings, TsconfigMode,
};
