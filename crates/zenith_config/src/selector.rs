//! Selectors, which pick the modules a contract is about, and ignored
//! imports, which pair two of them.
//!
//! A selector is written as a string:
//!
//! | Form | Matches |
//! |---|---|
//! | `web/src/domains/*` | internal directories and files, with a path [`Glob`] |
//! | `pkg:@acme/*` | the directories of matching workspace packages |
//! | `<scheme>:<name>` | the external packages or builtins of a language, such as `npm:@aws-sdk/*` |
//!
//! Package, external and builtin names are matched with a [`NamePattern`],
//! in which `*` also matches `/`. A selector can also be a table that removes
//! modules with `except`, or keeps only the files with a `directive`.
//!
//! The schemes other than `pkg:` belong to a [`Language`](crate::Language),
//! and need it to be enabled. [`Selector::parse`] and [`IgnoreImport::parse`]
//! check the syntax. Which modules a selector matches depends on the project,
//! so the contracts resolve them.

use std::fmt;

use zenith_utils::{
    glob::{Glob, NamePattern},
    location::ByteRange,
};

use crate::error::SelectorError;

/// A selector of modules, as written in the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// The selector as written, without surrounding whitespace.
    raw: String,

    /// What the selector matches.
    target: SelectorTarget,

    /// Selectors whose members are removed from each module.
    except: Vec<Selector>,

    /// The directive that files must have to be kept.
    directive: Option<String>,

    /// The byte range that the selector is written at in the configuration
    /// file.
    range: Option<ByteRange>,
}

/// What a selector matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorTarget {
    /// Internal directories and files whose path matches a glob.
    Path(Glob),

    /// The directories of the workspace packages whose name matches.
    Package(NamePattern),

    /// The external packages or builtins of a language's scheme whose name
    /// matches, such as `npm:react` or `node:fs`.
    Scheme {
        /// The scheme, without the `:`, which their node ids start with.
        scheme: &'static str,

        /// The pattern of names, without the scheme.
        pattern: NamePattern,
    },
}

impl Selector {
    /// A selector without filters, written as `raw`.
    pub(crate) fn new(raw: String, target: SelectorTarget) -> Self {
        Self { raw, target, except: Vec::new(), directive: None, range: None }
    }

    /// Record where the selector is written.
    pub(crate) fn at(mut self, range: ByteRange) -> Self {
        self.range = Some(range);
        self
    }

    /// Add the filters of the table form: modules to remove, and a directive
    /// that files must have.
    pub(crate) fn with_filters(
        mut self,
        except: Vec<Selector>,
        directive: Option<String>,
    ) -> Result<Self, SelectorError> {
        if directive.is_some() && !self.matches_files() {
            return Err(SelectorError::DirectiveWithoutFiles);
        }
        self.except = except;
        self.directive = directive;
        Ok(self)
    }

    /// The selector as written, e.g. `pkg:@acme/*`. For the table form, this
    /// is its `path`.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// What the selector matches.
    pub fn target(&self) -> &SelectorTarget {
        &self.target
    }

    /// The scheme the selector is written with, without the `:`, such as
    /// `pkg` or `npm`. `None` for a path.
    pub fn scheme(&self) -> Option<&'static str> {
        match &self.target {
            SelectorTarget::Path(_) => None,
            SelectorTarget::Package(_) => Some("pkg"),
            SelectorTarget::Scheme { scheme, .. } => Some(scheme),
        }
    }

    /// Selectors whose members are removed from each matched module.
    pub fn except(&self) -> &[Selector] {
        &self.except
    }

    /// The directive that a file must have in its prologue to be kept, e.g.
    /// `use client`. A selector with a directive stands for each such file
    /// on its own.
    pub fn directive(&self) -> Option<&str> {
        self.directive.as_deref()
    }

    /// The byte range that the selector is written at in the configuration
    /// file, for errors found when resolving it. `None` for a selector that
    /// wasn't read from a file. For the table form, this is the range of its
    /// `path`.
    pub fn range(&self) -> Option<ByteRange> {
        self.range
    }

    /// Whether matching nothing is a configuration error. Path and `pkg:`
    /// selectors must match something, to catch typos and renames. The
    /// selectors of a language's scheme may match nothing: it means nothing
    /// imports them.
    pub fn must_match(&self) -> bool {
        self.matches_files()
    }

    /// Whether the selector matches internal directories and files.
    fn matches_files(&self) -> bool {
        matches!(self.target, SelectorTarget::Path(_) | SelectorTarget::Package(_))
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// An `ignore_imports` entry, `"<importer> -> <imported>"`: the direct
/// imports from the importer selector's modules to the imported selector's
/// modules are ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreImport {
    /// The entry as written, without surrounding whitespace.
    raw: String,

    /// The importing modules.
    importer: Selector,

    /// The imported modules.
    imported: Selector,

    /// The byte range that the entry is written at in the configuration
    /// file.
    range: Option<ByteRange>,
}

impl IgnoreImport {
    /// An entry written as `raw`, from `importer` to `imported`.
    pub(crate) fn new(raw: String, importer: Selector, imported: Selector) -> Self {
        Self { raw, importer, imported, range: None }
    }

    /// Record where the entry, and so its selectors, is written.
    pub(crate) fn at(self, range: ByteRange) -> Self {
        Self {
            importer: self.importer.at(range),
            imported: self.imported.at(range),
            range: Some(range),
            ..self
        }
    }

    /// The entry as written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The importing modules.
    pub fn importer(&self) -> &Selector {
        &self.importer
    }

    /// The imported modules.
    pub fn imported(&self) -> &Selector {
        &self.imported
    }

    /// The byte range that the entry is written at in the configuration
    /// file, for reporting an entry that matches no import. `None` for an
    /// entry that wasn't read from a file.
    pub fn range(&self) -> Option<ByteRange> {
        self.range
    }
}

impl fmt::Display for IgnoreImport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::LANGUAGES;

    fn parse(raw: &str) -> Selector {
        Selector::parse(raw, LANGUAGES).unwrap()
    }

    #[test]
    fn only_path_and_package_selectors_must_match() {
        assert!(parse("web/src").must_match());
        assert!(parse("pkg:web").must_match());
        assert!(!parse("ext:sharp").must_match());
        assert!(!parse("core:*").must_match());
    }

    #[test]
    fn directives_only_apply_to_selectors_of_files() {
        let filter = |raw, directive: Option<&str>| {
            parse(raw).with_filters(Vec::new(), directive.map(String::from))
        };
        for raw in ["web/src", "pkg:web"] {
            assert!(filter(raw, Some("use client")).is_ok());
        }
        for raw in ["ext:react", "core:*"] {
            assert_eq!(filter(raw, Some("use client")), Err(SelectorError::DirectiveWithoutFiles));
            assert!(filter(raw, None).is_ok());
        }
    }
}
