//! Import kinds, which describe how a module is imported, and sets of them.

use std::fmt;

use bitflags::bitflags;
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

    /// An import that only tests compile, such as one in Rust's
    /// `#[cfg(test)]` code or in a crate's `tests` directory.
    Test,
}

impl ImportKind {
    /// Every import kind, in declaration order.
    pub const ALL: [ImportKind; 5] = [
        ImportKind::Value,
        ImportKind::Type,
        ImportKind::Dynamic,
        ImportKind::Require,
        ImportKind::Test,
    ];

    /// The kind of a static import or re-export: [`ImportKind::Type`] if it
    /// imports only types, and [`ImportKind::Value`] otherwise.
    pub const fn of_static(type_only: bool) -> ImportKind {
        if type_only { ImportKind::Type } else { ImportKind::Value }
    }

    /// The name of the kind as it appears in configuration and reports.
    pub const fn name(self) -> &'static str {
        match self {
            ImportKind::Value => "value",
            ImportKind::Type => "type",
            ImportKind::Dynamic => "dynamic",
            ImportKind::Require => "require",
            ImportKind::Test => "test",
        }
    }

    /// The bit that stands for this kind in a [`KindSet`].
    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

impl fmt::Display for ImportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

bitflags! {
    /// A set of [`ImportKind`]s, with one flag per kind. A single kind
    /// converts into the set that holds just it.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct KindSet: u8 {
        /// [`ImportKind::Value`].
        const VALUE = ImportKind::Value.bit();

        /// [`ImportKind::Type`].
        const TYPE = ImportKind::Type.bit();

        /// [`ImportKind::Dynamic`].
        const DYNAMIC = ImportKind::Dynamic.bit();

        /// [`ImportKind::Require`].
        const REQUIRE = ImportKind::Require.bit();

        /// [`ImportKind::Test`].
        const TEST = ImportKind::Test.bit();
    }
}

impl KindSet {
    /// Iterate over the kinds in the set, in the order of [`ImportKind::ALL`].
    pub fn kinds(self) -> impl Iterator<Item = ImportKind> {
        ImportKind::ALL.into_iter().filter(move |kind| self.contains((*kind).into()))
    }
}

impl From<ImportKind> for KindSet {
    fn from(kind: ImportKind) -> Self {
        KindSet::from_bits_retain(kind.bit())
    }
}

impl FromIterator<ImportKind> for KindSet {
    fn from_iter<T: IntoIterator<Item = ImportKind>>(iter: T) -> Self {
        iter.into_iter().map(KindSet::from).collect()
    }
}

impl Extend<ImportKind> for KindSet {
    fn extend<T: IntoIterator<Item = ImportKind>>(&mut self, iter: T) {
        self.extend(iter.into_iter().map(KindSet::from));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_own_flag() {
        // `KindSet` relies on `ALL` listing every kind once, in declaration
        // order, and on the kinds fitting in a `u8`.
        assert!(ImportKind::ALL.len() <= u8::BITS as usize);
        for (index, kind) in ImportKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, index);
        }
        let flags =
            [KindSet::VALUE, KindSet::TYPE, KindSet::DYNAMIC, KindSet::REQUIRE, KindSet::TEST];
        assert_eq!(ImportKind::ALL.map(KindSet::from), flags);
        assert_eq!(KindSet::all().kinds().collect::<Vec<_>>(), ImportKind::ALL);
    }

    #[test]
    fn static_imports_are_values_unless_type_only() {
        assert_eq!(ImportKind::of_static(true), ImportKind::Type);
        assert_eq!(ImportKind::of_static(false), ImportKind::Value);
    }

    #[test]
    fn names_match_the_configuration_spelling() {
        for kind in ImportKind::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.name()));
            assert_eq!(serde_json::from_str::<ImportKind>(&json).unwrap(), kind);
            assert_eq!(kind.to_string(), kind.name());
        }
        assert!(serde_json::from_str::<ImportKind>("\"Value\"").is_err());
    }

    #[test]
    fn kind_sets_hold_any_combination_of_kinds() {
        let mut set = KindSet::empty();
        assert!(set.is_empty());
        assert_eq!(set.kinds().count(), 0);

        set.extend([ImportKind::Require, ImportKind::Type, ImportKind::Type]);
        assert!(!set.is_empty());
        assert!(set.contains(ImportKind::Type.into()));
        assert!(set.contains(ImportKind::Require.into()));
        assert!(!set.contains(ImportKind::Value.into()));
        assert!(!set.contains(ImportKind::Dynamic.into()));
        assert_eq!(set.kinds().collect::<Vec<_>>(), [ImportKind::Type, ImportKind::Require]);
        assert_eq!(format!("{set:?}"), "KindSet(TYPE | REQUIRE)");
    }

    #[test]
    fn kind_sets_collect_from_kinds() {
        let set: KindSet = [ImportKind::Dynamic, ImportKind::Value].into_iter().collect();
        assert_eq!(set.kinds().collect::<Vec<_>>(), [ImportKind::Value, ImportKind::Dynamic]);

        let all: KindSet = ImportKind::ALL.into_iter().collect();
        assert_eq!(all, KindSet::all());
        assert_eq!(KindSet::default(), KindSet::empty());
    }
}
