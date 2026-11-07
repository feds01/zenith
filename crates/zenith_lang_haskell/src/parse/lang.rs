//! A [`HaskellFile`] as `zenith_lang` knows a file: its imports, each with
//! the specifier that the resolver reads back, its `zenith-ignore`
//! suppressions, and its syntax errors.

use std::fmt;

use zenith_graph::{ImportDetail, ImportKind, ImportedNames};
use zenith_lang::{Comment, CommentKind, ParsedFile, Suppression, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

use super::{HaskellFile, Import};

/// What an import names, as its specifier writes it: the module, after the
/// package that names the module's package, in quotes, as Haskell writes
/// them, and after `{-# SOURCE #-}` for an import of the module's boot file.
/// So `Data.Text`, `"text" Data.Text` and `{-# SOURCE #-} Acme.Types`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Specifier<'a> {
    /// The module, such as `Data.Text`.
    pub(crate) module: &'a str,

    /// The package that the import names, if it names one.
    pub(crate) package: Option<&'a str>,

    /// Whether the import is of the module's boot file.
    pub(crate) boot: bool,
}

/// What a specifier of an import of a boot file starts with.
const BOOT: &str = "{-# SOURCE #-} ";

impl<'a> Specifier<'a> {
    /// What the specifier `text` names.
    pub(crate) fn parse(text: &'a str) -> Self {
        let (boot, rest) = match text.strip_prefix(BOOT) {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (package, module) = match rest.strip_prefix('"').and_then(|rest| rest.split_once("\" "))
        {
            Some((package, module)) => (Some(package), module),
            None => (None, rest),
        };
        Self { module, package, boot }
    }
}

impl fmt::Display for Specifier<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.boot {
            f.write_str(BOOT)?;
        }
        if let Some(package) = self.package {
            write!(f, "\"{package}\" ")?;
        }
        f.write_str(self.module)
    }
}

impl Import {
    /// What it names, for its specifier.
    pub(crate) fn specifier(&self) -> Specifier<'_> {
        Specifier { module: &self.module, package: self.package.as_deref(), boot: self.source }
    }
}

impl HaskellFile {
    /// What the file, whose text is `source`, tells the pipeline.
    ///
    /// Every import is a value import of the whole module, located by the
    /// module's name. The suppressions are read from the comments that
    /// [`comments`](Self::comments) holds, which reach to the end of the
    /// imports, where imports are.
    pub(crate) fn into_parsed(self, source: &str) -> ParsedFile {
        let comments = (0..self.comments.len()).map(|index| self.lang_comment(index, source));
        let suppressions = Suppression::collect(source, comments);
        let imports = (self.imports.iter())
            .map(|import| ImportDetail {
                range: import.module_range,
                specifier: import.specifier().to_string().into(),
                kind: ImportKind::Value,
                names: ImportedNames::All,
            })
            .collect();
        let errors = (self.errors.into_iter())
            .map(|error| {
                Located::new(error.range, SyntaxError { message: error.value.to_string().into() })
            })
            .collect();
        ParsedFile { imports, suppressions, errors, ..ParsedFile::default() }
    }

    /// The comment at `index` of [`comments`](Self::comments), as finding
    /// suppressions needs it. Other comments next to it are not code.
    fn lang_comment(&self, index: usize, source: &str) -> Comment {
        let comment = &self.comments[index];
        let (start, end) = (comment.range.start() as usize, comment.range.end() as usize);

        // 1. Find what is between the delimiters: after the dashes of a line comment,
        //    or between the `{-` and the `-}` of a block comment.
        let text = &source[start..end];
        let (kind, content) = match comment.block {
            true => (CommentKind::Block, ByteRange::new((start + 2) as u32, (end - 2) as u32)),
            false => {
                let dashes = text.len() - text.trim_start_matches('-').len();
                (CommentKind::Line, ByteRange::new((start + dashes) as u32, end as u32))
            }
        };

        // 2. Code comes before the comment if it comes before the comments that lead up
        //    to it on its line.
        let mut first = start;
        for previous in self.comments[..index].iter().rev() {
            let (previous_start, previous_end) =
                (previous.range.start() as usize, previous.range.end() as usize);
            let between = &source[previous_end..first];
            if !between.trim().is_empty() || between.contains('\n') {
                break;
            }
            first = previous_start;
        }
        let line_before = source[..first].rsplit('\n').next().unwrap_or_default();

        // 3. The lexeme after the comment is the first thing after it that is not
        //    whitespace or another comment.
        let mut attached_to = end;
        let mut following = self.comments[index + 1..].iter();
        loop {
            let rest = &source[attached_to..];
            attached_to += rest.len() - rest.trim_start().len();
            match following.next() {
                Some(next) if next.range.start() as usize == attached_to => {
                    attached_to = next.range.end() as usize;
                }
                _ => break,
            }
        }

        let rest = source[end..].trim_start_matches([' ', '\t', '\r']);
        Comment {
            range: comment.range,
            content,
            kind,
            trailing: !line_before.trim().is_empty(),
            followed_by_newline: rest.is_empty() || rest.starts_with('\n'),
            attached_to: attached_to as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use zenith_utils::location::LineIndex;

    use super::*;
    use crate::parse::{Dialect, Parser};

    /// Parse `source` as a plain Haskell file.
    fn parse(source: &str) -> ParsedFile {
        Parser::new().parse(source, Dialect::PLAIN).into_parsed(source)
    }

    /// The suppressions in `source`, as `(contracts, reason, target line)`.
    fn suppressions(source: &str) -> Vec<(Vec<String>, Option<String>, u32)> {
        let parsed = parse(source);
        assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
        let lines = LineIndex::new(source);
        (parsed.suppressions.into_iter())
            .map(|suppression| {
                let Suppression { contracts, reason, target } = suppression.value;
                (contracts, reason, lines.line(target.start()))
            })
            .collect()
    }

    #[test]
    fn imports_are_value_imports_located_by_their_module() {
        let source = "import Data.Map (Map)\nimport qualified \"text\" Data.Text as T\nimport \
                      {-# SOURCE #-} Acme.Types\n";
        let parsed = parse(source);
        let imports: Vec<_> = (parsed.imports.iter())
            .map(|import| {
                (
                    &source[std::ops::Range::from(import.range)],
                    import.specifier.as_str(),
                    import.kind,
                )
            })
            .collect();
        assert_eq!(
            imports,
            [
                ("Data.Map", "Data.Map", ImportKind::Value),
                ("Data.Text", "\"text\" Data.Text", ImportKind::Value),
                ("Acme.Types", "{-# SOURCE #-} Acme.Types", ImportKind::Value),
            ]
        );
    }

    #[test]
    fn specifiers_are_read_back_as_they_are_written() {
        for (text, module, package, boot) in [
            ("Data.Map", "Data.Map", None, false),
            ("\"text\" Data.Text", "Data.Text", Some("text"), false),
            ("{-# SOURCE #-} Acme.Types", "Acme.Types", None, true),
            ("{-# SOURCE #-} \"acme\" Acme.Types", "Acme.Types", Some("acme"), true),
        ] {
            let specifier = Specifier::parse(text);
            assert_eq!(specifier, Specifier { module, package, boot });
            assert_eq!(specifier.to_string(), text);
        }
    }

    #[test]
    fn syntax_errors_keep_their_place_and_message() {
        let source = "import Data.Map (Map\nimport Data.Set\n";
        let parsed = parse(source);
        let errors: Vec<_> = parsed
            .errors
            .iter()
            .map(|error| (error.range.start(), error.value.to_string()))
            .collect();
        assert_eq!(errors, [(20, "expected `)`".to_owned())]);
        assert_eq!(parsed.imports.len(), 1);
    }

    #[test]
    fn comments_have_their_suppressions() {
        for (comment, contracts, reason) in [
            ("-- zenith-ignore api-layers", &["api-layers"][..], None),
            ("--zenith-ignore a, b,c", &["a", "b", "c"], None),
            ("--- zenith-ignore a: legacy", &["a"], Some("legacy")),
            ("{- zenith-ignore a: block -}", &["a"], Some("block")),
        ] {
            let source = format!("module M where\n\n{comment}\nimport B\n");
            let found = vec![(
                contracts.iter().map(|id| id.to_string()).collect(),
                reason.map(str::to_owned),
                4,
            )];
            assert_eq!(suppressions(&source), found, "{comment}");
        }
    }

    #[test]
    fn a_trailing_comment_suppresses_its_own_line() {
        let source = "module M where\nimport A -- zenith-ignore a\nimport B\n";
        assert_eq!(suppressions(source), [(vec!["a".to_owned()], None, 2)]);
    }
}
