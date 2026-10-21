//! A [`GoFile`] as `zenith_lang` knows a file: its imports, its `zenith-ignore`
//! suppressions, and its syntax errors.

use zenith_graph::{ImportDetail, ImportKind, ImportedNames};
use zenith_lang::{Comment, CommentKind, ParsedFile, Suppression, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

use super::GoFile;

impl GoFile {
    /// What the file, whose text is `source`, tells the pipeline.
    ///
    /// Every import is a value import of the whole package, located by its
    /// path. The suppressions are read from the comments that
    /// [`comments`](Self::comments) holds, which reach to the end of the
    /// imports, where imports are.
    pub(crate) fn into_parsed(self, source: &str) -> ParsedFile {
        let comments = (0..self.comments.len()).map(|index| self.lang_comment(index, source));
        let suppressions = Suppression::collect(source, comments);
        let imports = self
            .imports
            .into_iter()
            .map(|import| ImportDetail {
                range: import.path_range,
                specifier: import.path.into(),
                kind: ImportKind::Value,
                names: ImportedNames::All,
            })
            .collect();
        let errors = self
            .errors
            .into_iter()
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

        // 1. Find the delimiters. A block comment may be unterminated.
        let text = &source[start..end];
        let (kind, closing) = match comment.block {
            true => {
                (CommentKind::Block, if text.len() >= 4 && text.ends_with("*/") { 2 } else { 0 })
            }
            false => (CommentKind::Line, 0),
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

        // 3. The token after the comment is the first thing after it that is not
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
            content: ByteRange::new((start + 2) as u32, (end - closing) as u32),
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
    use crate::parse::Parser;

    /// Parse `source` as a Go file.
    fn parse(source: &str) -> ParsedFile {
        Parser::new().parse(source).into_parsed(source)
    }

    /// The suppressions in `source`, as `(contracts, reason, target line)`.
    fn suppressions(source: &str) -> Vec<(Vec<String>, Option<String>, u32)> {
        let parsed = parse(source);
        assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
        let lines = LineIndex::new(source);
        parsed
            .suppressions
            .into_iter()
            .map(|suppression| {
                let Suppression { contracts, reason, target } = suppression.value;
                (contracts, reason, lines.line(target.start()))
            })
            .collect()
    }

    fn found(
        contracts: &[&str],
        reason: Option<&str>,
        target_line: u32,
    ) -> Vec<(Vec<String>, Option<String>, u32)> {
        let contracts = contracts.iter().map(|id| id.to_string()).collect();
        vec![(contracts, reason.map(str::to_string), target_line)]
    }

    #[test]
    fn imports_are_value_imports_of_the_whole_package() {
        let source =
            "package a\n\nimport (\n\t\"fmt\"\n\tlog \"github.com/acme/log\"\n\t_ \"embed\"\n)\n";
        let parsed = parse(source);
        let imports: Vec<_> = parsed
            .imports
            .iter()
            .map(|import| {
                let range = std::ops::Range::from(import.range);
                (&source[range], import.specifier.as_str(), import.kind, &import.names)
            })
            .collect();
        assert_eq!(
            imports,
            [
                ("\"fmt\"", "fmt", ImportKind::Value, &ImportedNames::All),
                (
                    "\"github.com/acme/log\"",
                    "github.com/acme/log",
                    ImportKind::Value,
                    &ImportedNames::All
                ),
                ("\"embed\"", "embed", ImportKind::Value, &ImportedNames::All),
            ]
        );
    }

    #[test]
    fn syntax_errors_keep_their_place_and_message() {
        let source = "package a\n\nimport (\n\t\"fmt\"\n";
        let parsed = parse(source);
        let errors: Vec<_> = parsed
            .errors
            .iter()
            .map(|error| (error.range.start(), error.value.to_string()))
            .collect();
        assert_eq!(errors, [(source.len() as u32, "expected `)`".to_string())]);
        // The imports around the error are kept.
        assert_eq!(parsed.imports.len(), 1);
    }

    #[test]
    fn comments_have_their_suppressions() {
        for (comment, contracts, reason) in [
            ("// zenith-ignore api-layers", &["api-layers"][..], None),
            ("//zenith-ignore a, b,c", &["a", "b", "c"], None),
            ("// zenith-ignore a: legacy", &["a"], Some("legacy")),
            ("/* zenith-ignore a: block */", &["a"], Some("block")),
            ("/** zenith-ignore a */", &["a"], None),
        ] {
            let source = format!("package a\n\n{comment}\nimport \"b\"\n");
            assert_eq!(suppressions(&source), found(contracts, reason, 4), "{comment}");
        }
    }

    #[test]
    fn other_comments_are_not_suppressions() {
        for comment in ["// zenith-ignored a", "// TODO: zenith-ignore a", "/* zenith */", "/**/"] {
            let source = format!("package a\n\n{comment}\nimport \"b\"\n");
            assert_eq!(suppressions(&source), [], "{comment}");
        }
    }

    #[test]
    fn target_lines() {
        for (source, target_line) in [
            // A comment on a line of its own applies to the next line.
            ("package a\n// zenith-ignore a\nimport \"b\"\n", 3),
            ("package a\n  /* zenith-ignore a */  \nimport \"b\"\n", 3),
            // A trailing comment applies to its own line.
            ("package a\nimport \"b\" // zenith-ignore a\n", 2),
            ("package a\nimport \"b\" /* zenith-ignore a */\n", 2),
            ("package a\nimport (\n\t\"b\" // zenith-ignore a\n)\n", 3),
            ("package a // zenith-ignore a\nimport \"b\"\n", 1),
            // A comment before code applies to the line where it ends.
            ("package a\n/* zenith-ignore a */ import \"b\"\n", 2),
            ("package a\n/* zenith-ignore a\n */ import \"b\"\n", 3),
            // A comment over several lines applies to the line after its end.
            ("package a\n/*\n * zenith-ignore a\n */\nimport \"b\"\n", 5),
            // Other comments are not code.
            ("package a\n/* zenith-ignore a */ // why\nimport \"b\"\n", 3),
            ("package a\n/* note */ // zenith-ignore a\nimport \"b\"\n", 3),
            ("package a\nimport \"b\" /* x */ // zenith-ignore a\n", 2),
            // Lines end at `\n`, whatever comes before it.
            ("package a\r\n// zenith-ignore a\r\nimport \"b\"\r\n", 3),
            ("package a\r\nimport \"b\" // zenith-ignore a\r\n", 2),
        ] {
            assert_eq!(suppressions(source), found(&["a"], None, target_line), "{source:?}");
        }
    }
}
