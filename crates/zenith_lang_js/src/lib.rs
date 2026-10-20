//! JavaScript and TypeScript support: reading what each file imports and
//! exports with oxc, and resolving its specifiers.

mod resolve;

pub use resolve::{
    DEFAULT_CONDITIONS, DEFAULT_EXTENSIONS, DEFAULT_MAIN_FIELDS, Resolver, ResolverError,
    ResolverSettings, TsconfigMode,
};
