//! The libraries that a build installed, from their entries in its package
//! databases, as GHC reads them: each a `.conf` file, in the fields of
//! Cabal's syntax.

use super::{Installed, fields};

impl Installed {
    /// The library that `text`, an entry of a package database, records, or
    /// `None` if it names none.
    ///
    /// A library re-exports a module of another as
    /// `GHC.Num.Integer from ghc-bignum-1.3-627a:GHC.Num.Integer` among its
    /// exposed modules: the module is the library's, as it is what a
    /// component that imports it depends on.
    pub(crate) fn parse(text: &str) -> Option<Installed> {
        let items = fields::parse(text);
        let field = |name: &str| fields::fields(&items, name).next().map(str::trim);
        let id = field("id")?.to_owned();
        let package = field("package-name").or(field("name"))?.to_owned();
        let library = field("lib-name").map(str::to_owned);
        let mut exposed = Vec::new();
        let mut words = fields::fields(&items, "exposed-modules").flat_map(fields::words);
        while let Some(module) = words.next() {
            match module.as_str() {
                "from" => {
                    words.next();
                }
                _ => exposed.push(module),
            }
        }
        Some(Installed { id, package, library, exposed })
    }
}
