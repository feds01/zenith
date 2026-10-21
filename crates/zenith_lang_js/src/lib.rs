//! JavaScript and TypeScript support: reading what each file imports and
//! exports with oxc, resolving its specifiers, and the `[languages.js]` table
//! of the configuration.

mod backend;
mod config;
mod packages;
mod parse;
mod resolve;

pub use config::{JS, JsConfig, Tsconfig};
pub use packages::discover_packages;
pub use parse::{ParseOptions, Parser};
pub use resolve::{
    DEFAULT_CONDITIONS, DEFAULT_EXTENSIONS, DEFAULT_MAIN_FIELDS, Resolver, ResolverError,
    ResolverSettings, TsconfigMode,
};
