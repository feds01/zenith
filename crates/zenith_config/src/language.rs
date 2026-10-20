//! The languages that zenith reads, and what each one owns in the
//! configuration.
//!
//! Each language has a table of its own in `[languages]`, and is enabled by
//! writing it. The registry says, for each language, the names that its table
//! goes by, which files it scans by default, which extensions a path selector
//! may leave off its files, and which selector schemes, such as `npm:`, only
//! make sense with it. Everything that varies between languages is here, so
//! that the rest of the configuration doesn't need to know them.

/// Which language, for code that treats each differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageId {
    /// JavaScript and TypeScript.
    Js,
}

/// A language that zenith reads.
#[derive(Debug, PartialEq, Eq)]
pub struct Language {
    /// Which language it is.
    pub id: LanguageId,

    /// The name used in the docs and in errors, which is also the first of
    /// its [`table_names`](Self::table_names).
    pub name: &'static str,

    /// The names that its table in `[languages]` goes by. Giving its table
    /// under more than one of them is an error.
    pub table_names: &'static [&'static str],

    /// The globs of the files that it scans when its `include` is not set.
    pub include: &'static [&'static str],

    /// The extensions, without the `.`, of the files that
    /// [`include`](Self::include) matches. The file walk checks them instead
    /// of the globs, which is faster and finds the same files.
    pub source_extensions: &'static [&'static str],

    /// The extensions, without the `.`, that a path selector may leave off a
    /// file's path, so `src/types` matches the file `src/types.ts` as well as
    /// the directory `src/types`.
    pub optional_extensions: &'static [&'static str],

    /// The selector schemes, without the `:`, that only this language's
    /// projects have, so selectors that use them need the language to be
    /// enabled.
    pub schemes: &'static [&'static str],
}

/// JavaScript and TypeScript.
pub const JS: Language = Language {
    id: LanguageId::Js,
    name: "js",
    table_names: &["js", "typescript"],
    include: &["**/*.{ts,tsx,mts,cts,js,jsx,mjs,cjs}"],
    // Declaration files (`.d.ts`) are `ts` files.
    source_extensions: &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
    optional_extensions: &["ts", "tsx", "mts", "cts", "d.ts", "js", "jsx", "mjs", "cjs"],
    schemes: &["npm", "node", "bun"],
};

/// Every language, in the order that the docs list them.
pub const LANGUAGES: &[&Language] = &[&JS];

/// The selector schemes that don't belong to a language: `pkg:` selects
/// workspace packages, which any language's projects may have.
pub const NEUTRAL_SCHEMES: &[&str] = &["pkg"];

impl Language {
    /// The language whose table in `[languages]` goes by `name`.
    pub fn by_table_name(name: &str) -> Option<&'static Language> {
        LANGUAGES.iter().copied().find(|language| language.table_names.contains(&name))
    }

    /// The language that owns the selector scheme `scheme`, without the `:`.
    /// `None` for a scheme that no language owns, whether it is unknown or
    /// [neutral](NEUTRAL_SCHEMES).
    pub fn by_scheme(scheme: &str) -> Option<&'static Language> {
        LANGUAGES.iter().copied().find(|language| language.schemes.contains(&scheme))
    }
}

/// Every selector scheme, without the `:`: the neutral ones, then each
/// language's.
pub fn schemes() -> impl Iterator<Item = &'static str> {
    let owned = LANGUAGES.iter().flat_map(|language| language.schemes.iter());
    NEUTRAL_SCHEMES.iter().chain(owned).copied()
}

#[cfg(test)]
mod tests {
    use zenith_utils::glob::Glob;

    use super::*;

    #[test]
    fn every_language_is_named_by_its_first_table_name() {
        for language in LANGUAGES {
            assert_eq!(language.table_names.first(), Some(&language.name));
            for name in language.table_names {
                assert_eq!(Language::by_table_name(name), Some(*language), "{name}");
            }
        }
        assert_eq!(Language::by_table_name("go"), None);
    }

    #[test]
    fn table_names_belong_to_one_language() {
        let mut names: Vec<_> =
            LANGUAGES.iter().flat_map(|language| language.table_names).collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn schemes_belong_to_one_owner() {
        let mut all: Vec<_> = schemes().collect();
        let count = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), count);
        assert_eq!(Language::by_scheme("npm"), Some(&JS));
        assert_eq!(Language::by_scheme("pkg"), None);
    }

    #[test]
    fn the_source_extensions_are_what_the_default_include_matches() {
        for language in LANGUAGES {
            let globs: Vec<_> =
                language.include.iter().map(|glob| Glob::new(glob).unwrap()).collect();
            for extension in language.source_extensions {
                let path = format!("src/a.{extension}");
                assert!(globs.iter().any(|glob| glob.is_match(&path)), "{path}");
            }
            assert!(!globs.iter().any(|glob| glob.is_match("src/a.txt")));
        }
    }

    #[test]
    fn selectors_may_leave_off_every_source_extension() {
        for language in LANGUAGES {
            for extension in language.source_extensions {
                assert!(language.optional_extensions.contains(extension), "{extension}");
            }
        }
    }
}
