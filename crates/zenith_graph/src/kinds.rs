//! Import kinds, which describe how a module is imported.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How an import is made. Contracts can exclude imports of given kinds, e.g.
/// to exempt type-only imports.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ImportKind {
    /// A static value import or re-export: `import x from "s"`, `import "s"`,
    /// `export * from "s"`.
    Value,
    /// A type-only import that is erased at runtime: `import type`, an
    /// import whose specifiers are all `type`, or `typeof import("s")`.
    Type,
    /// A dynamic `import("s")` with a literal specifier.
    Dynamic,
    /// A CommonJS `require("s")` or TypeScript `import x = require("s")`.
    Require,
}

impl ImportKind {
    /// Every import kind.
    pub const ALL: [ImportKind; 4] =
        [ImportKind::Value, ImportKind::Type, ImportKind::Dynamic, ImportKind::Require];

    /// The name of the kind as it appears in configuration.
    pub fn name(self) -> &'static str {
        match self {
            ImportKind::Value => "value",
            ImportKind::Type => "type",
            ImportKind::Dynamic => "dynamic",
            ImportKind::Require => "require",
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

impl fmt::Display for ImportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of [ImportKind]s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KindSet(u8);

impl KindSet {
    /// The empty set.
    pub const EMPTY: KindSet = KindSet(0);

    /// Whether the set contains `kind`.
    pub fn contains(self, kind: ImportKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Add `kind` to the set.
    pub fn insert(&mut self, kind: ImportKind) {
        self.0 |= kind.bit();
    }

    /// Whether the set is empty.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Iterate over the kinds in the set.
    pub fn iter(self) -> impl Iterator<Item = ImportKind> {
        ImportKind::ALL.into_iter().filter(move |kind| self.contains(*kind))
    }
}

impl FromIterator<ImportKind> for KindSet {
    fn from_iter<T: IntoIterator<Item = ImportKind>>(iter: T) -> Self {
        let mut set = KindSet::EMPTY;
        for kind in iter {
            set.insert(kind);
        }
        set
    }
}
