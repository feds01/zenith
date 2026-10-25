//! Parsing the strings that selectors and ignored imports are written as, and
//! the selectors and ignored imports of a contract.

use zenith_utils::{
    glob::{Glob, NamePattern},
    location::ByteRange,
};

use super::Parser;
use crate::{
    error::{ConfigErrorKind, IgnoreImportError, SelectorError},
    language::Languages,
    schema::{Located, SelectorValue},
    selector::{IgnoreImport, Selector, SelectorTarget},
};

impl Selector {
    /// Parse the string form of a selector, which may use the scheme of any
    /// of the `languages`. Surrounding whitespace and a trailing `/` are
    /// ignored.
    pub fn parse(raw: &str, languages: Languages) -> Result<Self, SelectorError> {
        let raw = raw.trim();
        let target = if let Some((scheme, name)) = split_scheme(raw) {
            let pattern = || NamePattern::new(name);
            match languages.schemes().find(|known| *known == scheme) {
                Some("pkg") => SelectorTarget::Package(pattern()?),
                Some(scheme) => {
                    let paths = languages.by_scheme(scheme).map(|language| language.path_schemes);
                    let separator = paths.into_iter().flatten().find(|(owner, _)| *owner == scheme);
                    let pattern = match separator {
                        Some((_, separator)) => pattern()?.with_children(separator),
                        None => pattern()?,
                    };
                    SelectorTarget::Scheme { scheme, pattern }
                }
                None => return Err(SelectorError::unknown_scheme(scheme, languages)),
            }
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

/// Split a selector into its scheme, such as `npm`, and what follows the `:`.
/// `None` for a path, whose first segment is not a bare word followed by a
/// `:`.
fn split_scheme(raw: &str) -> Option<(&str, &str)> {
    let (scheme, name) = raw.split_once(':')?;
    let is_word = !scheme.is_empty()
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    is_word.then_some((scheme, name))
}

impl IgnoreImport {
    /// Parse an `ignore_imports` entry, `"<importer> -> <imported>"`, whose
    /// selectors may use the scheme of any of the `languages`.
    pub fn parse(raw: &str, languages: Languages) -> Result<Self, IgnoreImportError> {
        let raw = raw.trim();
        let mut sides = raw.split("->");
        let (Some(importer), Some(imported), None) = (sides.next(), sides.next(), sides.next())
        else {
            return Err(IgnoreImportError::Arrow);
        };
        let parse = |side: &str| {
            Selector::parse(side, languages).map_err(|error| IgnoreImportError::Selector {
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
        let selector = Selector::parse(raw, self.languages).map(|selector| selector.at(range));
        let selector = self.check(selector, Some(range), |error| {
            ConfigErrorKind::InvalidSelector { selector: raw.to_string(), error }
        })?;
        self.check_scheme(&selector, range).then_some(selector)
    }

    /// Whether the language that owns the scheme of `selector`, if it has
    /// one, is enabled. If not, record an error at `range`.
    pub(super) fn check_scheme(&mut self, selector: &Selector, range: ByteRange) -> bool {
        let Some(scheme) = selector.scheme() else {
            return true;
        };
        let Some(language) = self.languages.by_scheme(scheme) else {
            return true;
        };
        if self.enabled.contains(&language) {
            return true;
        }
        let kind = ConfigErrorKind::DisabledScheme {
            selector: selector.as_str().to_string(),
            scheme: scheme.to_string(),
            language: language.name.to_string(),
        };
        self.add_error(kind, Some(range));
        false
    }

    /// Parse `ignore_imports` entries, leaving out the invalid ones.
    pub(super) fn parse_ignore_imports(
        &mut self,
        entries: &[Located<String>],
    ) -> Vec<IgnoreImport> {
        entries
            .iter()
            .filter_map(|entry| {
                let ignore = IgnoreImport::parse(&entry.value, self.languages)
                    .map(|ignore| ignore.at(entry.range));
                let ignore = self.check(ignore, Some(entry.range), |error| {
                    ConfigErrorKind::InvalidIgnoreImport { entry: entry.value.clone(), error }
                })?;
                let importer = self.check_scheme(ignore.importer(), entry.range);
                let imported = self.check_scheme(ignore.imported(), entry.range);
                (importer && imported).then_some(ignore)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use zenith_utils::glob::GlobError;

    use super::*;
    use crate::testing::LANGUAGES;

    fn parse(raw: &str) -> Result<Selector, SelectorError> {
        Selector::parse(raw, LANGUAGES)
    }

    fn target(raw: &str) -> SelectorTarget {
        parse(raw).unwrap().target().clone()
    }

    fn parse_ignore(raw: &str) -> Result<IgnoreImport, IgnoreImportError> {
        IgnoreImport::parse(raw, LANGUAGES)
    }

    #[test]
    fn a_scheme_of_paths_matches_the_names_under_those_it_names() {
        let SelectorTarget::Scheme { pattern, .. } = target("core:fmt") else {
            panic!("expected a scheme");
        };
        assert!(pattern.is_match("fmt"));
        assert!(pattern.is_match("fmt::Display"));
        assert!(!pattern.is_match("fmtx"));
        // Names of other schemes are not paths.
        let SelectorTarget::Scheme { pattern, .. } = target("sys:fmt") else {
            panic!("expected a scheme");
        };
        assert!(!pattern.is_match("fmt::Display"));
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
            target("ext:@aws-sdk/*"),
            SelectorTarget::Scheme {
                scheme: "ext",
                pattern: NamePattern::new("@aws-sdk/*").unwrap()
            }
        );
        assert_eq!(
            target("oth:*"),
            SelectorTarget::Scheme { scheme: "oth", pattern: NamePattern::new("*").unwrap() }
        );
        // Next.js and TanStack file names need no escaping.
        assert!(matches!(
            target("web/src/app/(dashboard)/[id]/$slug.tsx"),
            SelectorTarget::Path(_)
        ));
    }

    #[test]
    fn ignores_surrounding_whitespace_and_a_trailing_slash() {
        let selector = parse("  web/src/ ").unwrap();
        assert_eq!(selector.as_str(), "web/src/");
        assert_eq!(selector.target(), &SelectorTarget::Path(Glob::new("web/src").unwrap()));
        assert_eq!(selector.to_string(), "web/src/");
    }

    #[test]
    fn rejects_invalid_selectors() {
        for (raw, error) in [
            ("", SelectorError::Pattern(GlobError::Empty)),
            ("pkg:", SelectorError::Pattern(GlobError::Empty)),
            ("ext:", SelectorError::Pattern(GlobError::Empty)),
            ("/abs", SelectorError::Pattern(GlobError::LeadingSlash)),
            ("web//src", SelectorError::Pattern(GlobError::EmptySegment)),
            ("web/src**", SelectorError::Pattern(GlobError::DoubleStar)),
            ("web/{a,b", SelectorError::Pattern(GlobError::UnclosedBrace)),
            ("ext:@acme/{a", SelectorError::Pattern(GlobError::UnclosedBrace)),
            ("../up", SelectorError::ParentDir),
            ("web/../../etc", SelectorError::ParentDir),
        ] {
            assert_eq!(parse(raw), Err(error), "{raw:?}");
        }
    }

    #[test]
    fn rejects_unknown_schemes() {
        let error = parse("exy:react").unwrap_err();
        assert_eq!(error.to_string(), "unknown scheme `exy:`; did you mean `ext:`?");
        let error = parse("gopkg:fmt").unwrap_err();
        assert_eq!(
            error.to_string(),
            "unknown scheme `gopkg:`, expected one of `pkg:`, `ext:`, `core:`, `sys:`, `oth:`"
        );
        // Only a bare word before the `:` is a scheme.
        for raw in ["web/src/a:b", "{a,b}:c", ":a", "a b:c"] {
            assert!(matches!(target(raw), SelectorTarget::Path(_)), "{raw}");
        }
    }

    #[test]
    fn every_scheme_of_the_registry_is_parsed() {
        for scheme in LANGUAGES.schemes() {
            let selector = parse(&format!("{scheme}:x")).unwrap();
            assert_eq!(selector.scheme(), Some(scheme));
        }
    }

    #[test]
    fn parses_ignored_imports() {
        let ignore = parse_ignore(" web/src/features/* -> pkg:@acme/shared ").unwrap();
        assert_eq!(ignore.as_str(), "web/src/features/* -> pkg:@acme/shared");
        assert_eq!(ignore.importer().as_str(), "web/src/features/*");
        assert_eq!(ignore.imported().as_str(), "pkg:@acme/shared");
        assert!(parse_ignore("web/src->ext:react").is_ok());
    }

    #[test]
    fn rejects_invalid_ignored_imports() {
        assert_eq!(parse_ignore("web/src"), Err(IgnoreImportError::Arrow));
        assert_eq!(parse_ignore("a -> b -> c"), Err(IgnoreImportError::Arrow));
        assert_eq!(
            parse_ignore("web/src -> /abs"),
            Err(IgnoreImportError::Selector {
                selector: "/abs".into(),
                error: SelectorError::Pattern(GlobError::LeadingSlash)
            })
        );
        assert_eq!(
            parse_ignore(" -> web").unwrap_err().to_string(),
            "invalid selector ``: the pattern is empty"
        );
    }
}
