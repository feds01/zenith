//! Parsing the strings that selectors and ignored imports are written as, and
//! the selectors and ignored imports of a contract.

use zenith_utils::{
    glob::{Glob, NamePattern},
    location::ByteRange,
};

use super::Parser;
use crate::{
    error::{ConfigErrorKind, IgnoreImportError, SelectorError},
    schema::{Located, SelectorValue},
    selector::{BuiltinScheme, IgnoreImport, Selector, SelectorTarget},
};

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
        Ok(Self::new(raw.to_string(), target))
    }
}

impl IgnoreImport {
    /// Parse an `ignore_imports` entry, `"<importer> -> <imported>"`.
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
        Ok(Self::new(raw.to_string(), parse(importer)?, parse(imported)?))
    }
}

impl Parser {
    /// Parse a list of selectors, leaving out the invalid ones.
    pub(super) fn parse_selectors(&mut self, values: &[Located<SelectorValue>]) -> Vec<Selector> {
        values.iter().filter_map(|value| self.parse_selector(value)).collect()
    }

    /// Parse a selector, in its string or table form.
    fn parse_selector(&mut self, value: &Located<SelectorValue>) -> Option<Selector> {
        let table = match &value.value {
            SelectorValue::String(raw) => return self.parse_string_selector(raw, value.range),
            SelectorValue::Table(table) => table,
        };
        let selector = self.parse_string_selector(&table.path.value, table.path.range);
        let except = table
            .except
            .iter()
            .filter_map(|except| self.parse_string_selector(&except.value, except.range))
            .collect();
        let directive = table.directive.as_ref();
        let selector =
            selector?.with_filters(except, directive.map(|directive| directive.value.clone()));
        self.check(selector, directive.map(|directive| directive.range), |error| {
            ConfigErrorKind::InvalidSelector { selector: table.path.value.clone(), error }
        })
    }

    /// Parse the string form of a selector, written at `range`.
    fn parse_string_selector(&mut self, raw: &str, range: ByteRange) -> Option<Selector> {
        let selector = Selector::parse(raw).map(|selector| selector.at(range));
        self.check(selector, Some(range), |error| ConfigErrorKind::InvalidSelector {
            selector: raw.to_string(),
            error,
        })
    }

    /// Parse `ignore_imports` entries, leaving out the invalid ones.
    pub(super) fn parse_ignore_imports(
        &mut self,
        entries: &[Located<String>],
    ) -> Vec<IgnoreImport> {
        entries
            .iter()
            .filter_map(|entry| {
                let ignore = IgnoreImport::parse(&entry.value).map(|ignore| ignore.at(entry.range));
                self.check(ignore, Some(entry.range), |error| {
                    ConfigErrorKind::InvalidIgnoreImport { entry: entry.value.clone(), error }
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use zenith_utils::glob::GlobError;

    use super::*;

    fn target(raw: &str) -> SelectorTarget {
        Selector::parse(raw).unwrap().target().clone()
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
        assert_eq!(selector.target(), &SelectorTarget::Path(Glob::new("web/src").unwrap()));
        assert_eq!(selector.to_string(), "web/src/");
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
