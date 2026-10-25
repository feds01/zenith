//! What the configuration needs from each language that zenith reads.
//!
//! Each language has a table of its own in `[languages]`, and is enabled by
//! writing it. A [`Language`] says the names that its table goes by, which
//! files it scans by default, which extensions a path selector may leave off
//! its files, which selector schemes, such as `npm:`, only make sense with
//! it, and how to parse and describe its table. The languages themselves are
//! defined in their own crates, and given to [`Config::parse`] as
//! [`Languages`], so this crate knows none of them.
//!
//! [`Config::parse`]: crate::Config::parse

use std::{any::Any, fmt, sync::Arc};

use schemars::{Schema, SchemaGenerator};
use zenith_lang::{Backend, BackendSettings};
use zenith_utils::glob::Glob;

use crate::parse::LanguageTable;

/// A language that zenith reads.
pub struct Language {
    /// The name used in the docs and in errors, which is also the first of
    /// its [`table_names`](Self::table_names).
    pub name: &'static str,

    /// The names that its table in `[languages]` goes by. Giving its table
    /// under more than one of them is an error.
    pub table_names: &'static [&'static str],

    /// What the language is, for the description of its table in the
    /// schema.
    pub description: &'static str,

    /// Whether it is enabled when the configuration has no `[languages]`
    /// table.
    pub enabled_by_default: bool,

    /// The globs of the files that it scans when its `include` is not set.
    pub include: &'static [&'static str],

    /// The extensions, without the `.`, of the files that
    /// [`include`](Self::include) matches. The file walk checks them instead
    /// of the globs, which is faster and finds the same files.
    pub source_extensions: &'static [&'static str],

    /// The names of the files that [`include`](Self::include) matches besides
    /// those with a source extension, such as `terragrunt.hcl`, which the file
    /// walk checks instead of the globs too.
    pub source_file_names: &'static [&'static str],

    /// The extensions, without the `.`, that a path selector may leave off a
    /// file's path, so `src/types` matches the file `src/types.ts` as well as
    /// the directory `src/types`.
    pub optional_extensions: &'static [&'static str],

    /// The names of the files that are the module of the directory they are
    /// in, or the root of a target, rather than a module in it, such as
    /// Rust's `mod.rs` and `lib.rs`. An exhaustive `layers` contract doesn't
    /// count them as children of its containers.
    pub index_files: &'static [&'static str],

    /// The selector schemes, without the `:`, that only this language's
    /// projects have, so selectors that use them need the language to be
    /// enabled.
    pub schemes: &'static [&'static str],

    /// Parse its table into its options. An invalid table has the defaults,
    /// so that the language is still enabled.
    pub parse: fn(&mut LanguageTable<'_, '_>) -> Arc<dyn LanguageOptions>,

    /// Its options when it is enabled without a table.
    pub defaults: fn() -> Arc<dyn LanguageOptions>,

    /// The JSON Schema of its table.
    pub schema: fn(&mut SchemaGenerator) -> Schema,
}

/// Languages are told apart by name, which is unique.
impl PartialEq for Language {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Language {}

impl fmt::Debug for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Language").field(&self.name).finish()
    }
}

/// The options of a language, parsed from its table.
pub trait LanguageOptions: Any + fmt::Debug + Send + Sync {
    /// Globs of the files to scan, or `None` for every file with one of the
    /// language's [source extensions](Language::source_extensions) or
    /// [source file names](Language::source_file_names).
    fn include(&self) -> Option<&[Glob]>;

    /// The language, set up by these options for the pipeline to run.
    fn backend(&self, settings: BackendSettings<'_>) -> Box<dyn Backend>;
}

/// The languages that a configuration may enable, in the order that the
/// docs list them.
#[derive(Debug, Clone, Copy)]
pub struct Languages(&'static [&'static Language]);

/// The selector schemes that don't belong to a language: `pkg:` selects
/// workspace packages, which any language's projects may have.
pub const NEUTRAL_SCHEMES: &[&str] = &["pkg"];

impl Languages {
    /// The languages in `languages`.
    pub const fn new(languages: &'static [&'static Language]) -> Self {
        Self(languages)
    }

    /// Each language.
    pub fn iter(self) -> impl Iterator<Item = &'static Language> {
        self.0.iter().copied()
    }

    /// The language whose table in `[languages]` goes by `name`.
    pub fn by_table_name(self, name: &str) -> Option<&'static Language> {
        self.iter().find(|language| language.table_names.contains(&name))
    }

    /// The language that owns the selector scheme `scheme`, without the `:`.
    /// `None` for a scheme that no language owns, whether it is unknown or
    /// [neutral](NEUTRAL_SCHEMES).
    pub fn by_scheme(self, scheme: &str) -> Option<&'static Language> {
        self.iter().find(|language| language.schemes.contains(&scheme))
    }

    /// Every name that a language's table goes by.
    pub fn table_names(self) -> impl Iterator<Item = &'static str> {
        self.iter().flat_map(|language| language.table_names.iter().copied())
    }

    /// Every selector scheme, without the `:`: the neutral ones, then each
    /// language's.
    pub fn schemes(self) -> impl Iterator<Item = &'static str> {
        let owned = self.iter().flat_map(|language| language.schemes.iter().copied());
        NEUTRAL_SCHEMES.iter().copied().chain(owned)
    }
}
