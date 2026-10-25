//! What an import statement takes from the module it imports, and what a
//! module exports, so that an import from a pass-through module, a file that
//! only re-exports other modules, can be followed to the modules that define
//! what it takes.

use compact_str::CompactString;
use thin_vec::ThinVec;

/// What an import statement takes from the module it imports.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ImportedNames {
    /// The whole module: a namespace import (`import * as ns`), a side-effect
    /// import, `export *`, a dynamic import or a `require` call, whose use of
    /// the module can't be told from the statement.
    #[default]
    All,

    /// Only these exports, e.g. `["default", "a"]` for
    /// `import b, { a } from "./m"`, in source order. Never empty: an import of
    /// no names (`import {} from "./m"`) runs the module, so it takes
    /// `All`.
    Names(ThinVec<ImportedName>),
}

impl ImportedNames {
    /// The names, or `All` if there are none, which an import of no names
    /// takes.
    pub fn of(names: ThinVec<ImportedName>) -> Self {
        match names.is_empty() {
            true => Self::All,
            false => Self::Names(names),
        }
    }
}

/// An export that an import statement takes by name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImportedName {
    /// The name the module exports it as: `default` for a default import.
    pub name: CompactString,

    /// Whether only its type is imported, as with `import { type A }` or
    /// `import type { A }`.
    pub type_only: bool,
}

impl ImportedName {
    /// The export called `name`, imported as a value unless `type_only`.
    pub fn new(name: impl Into<CompactString>, type_only: bool) -> Self {
        Self { name: name.into(), type_only }
    }
}

/// What a module exports. `S` stands for the source module of each re-export:
/// the index of its import statement in the file, while the file is parsed,
/// or the node it resolved to, in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exports<S> {
    /// Whether the module is a pass-through module, from which importing a
    /// name is importing the name from where it is defined: every statement
    /// in it imports or re-exports, so that it has no code of its own, or it
    /// is a module of a language whose modules run no code when they are
    /// imported, such as Rust. The names that it defines itself, in
    /// [`names`](Self::names), it defines.
    pub pass_through: bool,

    /// The names that the module exports from its own code, e.g. `a` for
    /// `export const a = 1`, `default` for `export default …`.
    pub names: ThinVec<ImportedName>,

    /// The names that the module exports from other modules.
    pub reexports: ThinVec<ReExport<S>>,

    /// The modules whose every named export the module exports too, with
    /// `export * from`, and whether only their types.
    pub stars: ThinVec<(S, bool)>,
}

impl<S> Default for Exports<S> {
    /// A module of code that exports nothing, as a file that can't be parsed
    /// is taken to be.
    fn default() -> Self {
        Self {
            pass_through: false,
            names: ThinVec::new(),
            reexports: ThinVec::new(),
            stars: ThinVec::new(),
        }
    }
}

impl<S> Exports<S> {
    /// These exports, with the source of each re-export mapped by `source`.
    /// A re-export or star whose source maps to `None` is left out, as one
    /// whose import couldn't be resolved.
    pub fn map<T>(self, mut source: impl FnMut(S) -> Option<T>) -> Exports<T> {
        Exports {
            pass_through: self.pass_through,
            names: self.names,
            reexports: self
                .reexports
                .into_iter()
                .filter_map(|reexport| {
                    Some(ReExport {
                        name: reexport.name,
                        source: source(reexport.source)?,
                        imported: reexport.imported,
                        type_only: reexport.type_only,
                    })
                })
                .collect(),
            stars: self
                .stars
                .into_iter()
                .filter_map(|(star, type_only)| Some((source(star)?, type_only)))
                .collect(),
        }
    }
}

/// A name that a module exports from another module, its source, as with
/// `export { a as b } from "./m"`, or `import { a } from "./m"` then
/// `export { a as b }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReExport<S> {
    /// The name the module exports it as: `b` in the examples.
    pub name: CompactString,

    /// The module it comes from.
    pub source: S,

    /// The name the source exports it as, `a` in the examples, or `None` for
    /// the whole source module, as with `export * as b from "./m"`.
    pub imported: Option<CompactString>,

    /// Whether only its type is exported, as with `export type { a }`.
    pub type_only: bool,
}
