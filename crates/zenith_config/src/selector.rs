//! Selectors, which pick the modules a contract is about, and ignored
//! imports, which pair two of them.
//!
//! A selector is written as a string:
//!
//! | Form | Matches |
//! |---|---|
//! | `web/src/domains/*` | internal directories and files, with a path [`Glob`] |
//! | `pkg:@acme/*` | the directories of matching workspace packages |
//! | `npm:@aws-sdk/*` | external packages |
//! | `node:*`, `bun:*` | builtins |
//!
//! Package, external and builtin names are matched with a [`NamePattern`],
//! in which `*` also matches `/`. A selector can also be a table that removes
//! modules with `except`, or keeps only the files with a `directive`.
//!
//! This module only checks the syntax. Which modules a selector matches
//! depends on the project, so the contracts resolve them.

use std::fmt;

use zenith_utils::glob::{Glob, GlobError, NamePattern};

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
}

/// What a selector matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorTarget {
    /// Internal directories and files whose path matches a glob.
    Path(Glob),

    /// The directories of the workspace packages whose name matches.
    Package(NamePattern),

    /// The external packages whose name matches.
    External(NamePattern),

    /// The builtins of a runtime whose name matches.
    Builtin {
        /// The runtime the builtins belong to.
        scheme: BuiltinScheme,

        /// The pattern of builtin names, without the scheme.
        pattern: NamePattern,
    },
}

/// The runtime that a builtin selector refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinScheme {
    /// Node builtins, `node:<name>`.
    Node,

    /// Bun builtins, `bun:<name>`.
    Bun,
}

impl BuiltinScheme {
    /// The scheme as written before the `:`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Bun => "bun",
        }
    }
}

/// Why a selector is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorError {
    /// The glob or name pattern is invalid.
    Pattern(GlobError),

    /// A path goes up with `..`, out of the project root.
    ParentDir,

    /// A `directive` is set on a selector that matches no files.
    DirectiveWithoutFiles,
}

impl fmt::Display for SelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pattern(error) => write!(f, "{error}"),
            Self::ParentDir => {
                f.write_str("`..` is not allowed; paths are relative to the project root")
            }
            Self::DirectiveWithoutFiles => f.write_str(
                "`directive` only applies to path and `pkg:` selectors, which match files",
            ),
        }
    }
}

impl std::error::Error for SelectorError {}

impl From<GlobError> for SelectorError {
    fn from(error: GlobError) -> Self {
        Self::Pattern(error)
    }
}

impl Selector {
    /// Parse the string form of a selector. Surrounding whitespace and a
    /// trailing `/` are ignored.
    pub fn parse(raw: &str) -> Result<Self, SelectorError> {
        let raw = raw.trim();
        let target = if let Some(name) = raw.strip_prefix("pkg:") {
            SelectorTarget::Package(NamePattern::new(name)?)
        } else if let Some(name) = raw.strip_prefix("npm:") {
            SelectorTarget::External(NamePattern::new(name)?)
        } else if let Some(name) = raw.strip_prefix("node:") {
            SelectorTarget::Builtin {
                scheme: BuiltinScheme::Node,
                pattern: NamePattern::new(name)?,
            }
        } else if let Some(name) = raw.strip_prefix("bun:") {
            SelectorTarget::Builtin { scheme: BuiltinScheme::Bun, pattern: NamePattern::new(name)? }
        } else {
            let path = raw.strip_suffix('/').unwrap_or(raw);
            if path.split('/').any(|segment| segment == "..") {
                return Err(SelectorError::ParentDir);
            }
            SelectorTarget::Path(Glob::new(path)?)
        };
        Ok(Self { raw: raw.to_string(), target, except: Vec::new(), directive: None })
    }

    /// Add the filters of the table form: modules to remove, and a directive
    /// that files must have.
    pub fn with_filters(
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

    /// Whether matching nothing is a configuration error. Path and `pkg:`
    /// selectors must match something, to catch typos and renames. `npm:`
    /// and builtin selectors may match nothing: it means nothing imports
    /// them.
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
}

/// Why an `ignore_imports` entry is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IgnoreImportError {
    /// The entry doesn't have exactly one `->`.
    Arrow,

    /// One of the two selectors is invalid.
    Selector {
        /// The selector as written.
        selector: String,

        /// What is wrong with it.
        error: SelectorError,
    },
}

impl fmt::Display for IgnoreImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arrow => f.write_str("expected `<importer> -> <imported>`"),
            Self::Selector { selector, error } => {
                write!(f, "invalid selector `{selector}`: {error}")
            }
        }
    }
}

impl std::error::Error for IgnoreImportError {}

impl IgnoreImport {
    /// Parse an `ignore_imports` entry.
    pub fn parse(raw: &str) -> Result<Self, IgnoreImportError> {
        let raw = raw.trim();
        let mut sides = raw.split("->");
        let (Some(importer), Some(imported), None) = (sides.next(), sides.next(), sides.next())
        else {
            return Err(IgnoreImportError::Arrow);
        };
        let parse = |side: &str| {
            Selector::parse(side).map_err(|error| IgnoreImportError::Selector {
                selector: side.trim().to_string(),
                error,
            })
        };
        Ok(Self { raw: raw.to_string(), importer: parse(importer)?, imported: parse(imported)? })
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
}

impl fmt::Display for IgnoreImport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(raw: &str) -> SelectorTarget {
        Selector::parse(raw).unwrap().target
    }

    #[test]
    fn parses_every_string_form() {
        assert_eq!(
            target("web/src/domains/*"),
            SelectorTarget::Path(Glob::new("web/src/domains/*").unwrap())
        );
        assert_eq!(
            target("pkg:@acme/*"),
            SelectorTarget::Package(NamePattern::new("@acme/*").unwrap())
        );
        assert_eq!(
            target("npm:@aws-sdk/*"),
            SelectorTarget::External(NamePattern::new("@aws-sdk/*").unwrap())
        );
        assert_eq!(
            target("node:fs"),
            SelectorTarget::Builtin {
                scheme: BuiltinScheme::Node,
                pattern: NamePattern::new("fs").unwrap()
            }
        );
        assert_eq!(
            target("bun:*"),
            SelectorTarget::Builtin {
                scheme: BuiltinScheme::Bun,
                pattern: NamePattern::new("*").unwrap()
            }
        );
        // Next.js and TanStack file names need no escaping.
        assert!(matches!(
            target("web/src/app/(dashboard)/[id]/$slug.tsx"),
            SelectorTarget::Path(_)
        ));
    }

    #[test]
    fn ignores_surrounding_whitespace_and_a_trailing_slash() {
        let selector = Selector::parse("  web/src/ ").unwrap();
        assert_eq!(selector.as_str(), "web/src/");
        assert_eq!(selector.target, SelectorTarget::Path(Glob::new("web/src").unwrap()));
        assert_eq!(selector.to_string(), "web/src/");
    }

    #[test]
    fn only_path_and_package_selectors_must_match() {
        assert!(Selector::parse("web/src").unwrap().must_match());
        assert!(Selector::parse("pkg:web").unwrap().must_match());
        assert!(!Selector::parse("npm:sharp").unwrap().must_match());
        assert!(!Selector::parse("node:*").unwrap().must_match());
        assert!(!Selector::parse("bun:sqlite").unwrap().must_match());
    }

    #[test]
    fn rejects_invalid_selectors() {
        for (raw, error) in [
            ("", SelectorError::Pattern(GlobError::Empty)),
            ("pkg:", SelectorError::Pattern(GlobError::Empty)),
            ("npm:", SelectorError::Pattern(GlobError::Empty)),
            ("/abs", SelectorError::Pattern(GlobError::LeadingSlash)),
            ("web//src", SelectorError::Pattern(GlobError::EmptySegment)),
            ("web/src**", SelectorError::Pattern(GlobError::DoubleStar)),
            ("web/{a,b", SelectorError::Pattern(GlobError::UnclosedBrace)),
            ("npm:@acme/{a", SelectorError::Pattern(GlobError::UnclosedBrace)),
            ("../up", SelectorError::ParentDir),
            ("web/../../etc", SelectorError::ParentDir),
        ] {
            assert_eq!(Selector::parse(raw), Err(error), "{raw:?}");
        }
    }

    #[test]
    fn directives_only_apply_to_selectors_of_files() {
        let filter = |raw, directive: Option<&str>| {
            Selector::parse(raw).unwrap().with_filters(Vec::new(), directive.map(String::from))
        };
        for raw in ["web/src", "pkg:web"] {
            assert!(filter(raw, Some("use client")).is_ok());
        }
        for raw in ["npm:react", "node:*"] {
            assert_eq!(filter(raw, Some("use client")), Err(SelectorError::DirectiveWithoutFiles));
            assert!(filter(raw, None).is_ok());
        }
    }

    #[test]
    fn parses_ignored_imports() {
        let ignore = IgnoreImport::parse(" web/src/features/* -> pkg:@acme/shared ").unwrap();
        assert_eq!(ignore.as_str(), "web/src/features/* -> pkg:@acme/shared");
        assert_eq!(ignore.importer().as_str(), "web/src/features/*");
        assert_eq!(ignore.imported().as_str(), "pkg:@acme/shared");
        assert!(IgnoreImport::parse("web/src->npm:react").is_ok());
    }

    #[test]
    fn rejects_invalid_ignored_imports() {
        assert_eq!(IgnoreImport::parse("web/src"), Err(IgnoreImportError::Arrow));
        assert_eq!(IgnoreImport::parse("a -> b -> c"), Err(IgnoreImportError::Arrow));
        assert_eq!(
            IgnoreImport::parse("web/src -> /abs"),
            Err(IgnoreImportError::Selector {
                selector: "/abs".into(),
                error: SelectorError::Pattern(GlobError::LeadingSlash)
            })
        );
        assert_eq!(
            IgnoreImport::parse(" -> web").unwrap_err().to_string(),
            "invalid selector ``: the pattern is empty"
        );
    }
}
